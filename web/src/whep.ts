const NEGOTIATION_TIMEOUT_MS = 10_000;

/** One direct browser-to-MediaMTX WHEP session. */
export interface WhepSession {
  /** Attaches the negotiated track only after the caller accepts its generation. */
  attach(video: HTMLVideoElement): void;
  close(): Promise<void>;
}

/**
 * Negotiates a receive-only track without mutating the visible video element.
 * Cancellation and the bounded negotiation deadline close all local peer work.
 */
export async function startWhep(
  endpoint: string,
  onFailure: () => void,
  cancellation: AbortSignal
): Promise<WhepSession> {
  const peer = new RTCPeerConnection();
  peer.addTransceiver("video", { direction: "recvonly" });
  let negotiatedStream: MediaStream | null = null;
  const timeout = new AbortController();
  const timeoutTimer = window.setTimeout(() => {
    timeout.abort(new DOMException("WHEP negotiation timed out", "TimeoutError"));
  }, NEGOTIATION_TIMEOUT_MS);
  const negotiation = new AbortController();
  const signal = AbortSignal.any([cancellation, timeout.signal, negotiation.signal]);
  let sessionUrl: string | null = null;

  try {
    signal.throwIfAborted();
    const offer = await raceWithAbort(peer.createOffer(), signal);
    await raceWithAbort(peer.setLocalDescription(offer), signal);
    await waitForIceGathering(peer, signal);
    const localDescription = peer.localDescription;
    if (localDescription === null) {
      throw new Error("WHEP offer was not established");
    }

    const response = await fetch(endpoint, {
      method: "POST",
      headers: { "Content-Type": "application/sdp" },
      body: localDescription.sdp,
      signal
    });
    if (response.status !== 201) {
      throw new Error(`WHEP request failed with ${response.status}`);
    }
    const resource = response.headers.get("Location");
    if (resource === null) {
      throw new Error("WHEP response omitted its session resource");
    }
    const createdSessionUrl = new URL(resource, endpoint).toString();
    sessionUrl = createdSessionUrl;
    const track = waitForTrack(peer, signal);
    const installRemoteDescription = async (): Promise<void> => {
      const answer = await raceWithAbort(response.text(), signal);
      await raceWithAbort(peer.setRemoteDescription({ type: "answer", sdp: answer }), signal);
    };
    [negotiatedStream] = await Promise.all([track, installRemoteDescription()]);
    let closed = false;
    let failureReported = false;
    const connectionFailed = (): void => {
      if (!closed && !failureReported && peer.connectionState === "failed") {
        failureReported = true;
        onFailure();
      }
    };
    peer.addEventListener("connectionstatechange", connectionFailed);
    connectionFailed();
    let cleanup: Promise<void> | null = null;

    return {
      attach(video: HTMLVideoElement): void {
        if (closed || negotiatedStream === null) return;
        video.srcObject = negotiatedStream;
      },
      close(): Promise<void> {
        if (!closed) {
          closed = true;
          peer.close();
          cleanup = cleanupSession(createdSessionUrl);
        }
        return cleanup ?? Promise.resolve();
      }
    };
  } catch (error: unknown) {
    negotiation.abort(error);
    peer.close();
    if (sessionUrl !== null) {
      void cleanupSession(sessionUrl);
    }
    throw error;
  } finally {
    window.clearTimeout(timeoutTimer);
  }
}

function raceWithAbort<T>(operation: Promise<T>, signal: AbortSignal): Promise<T> {
  signal.throwIfAborted();
  return new Promise<T>((resolve, reject) => {
    const aborted = (): void => {
      reject(signal.reason);
    };
    signal.addEventListener("abort", aborted, { once: true });
    operation.then(
      (value) => {
        signal.removeEventListener("abort", aborted);
        resolve(value);
      },
      (error: unknown) => {
        signal.removeEventListener("abort", aborted);
        reject(error);
      }
    );
  });
}

function cleanupSession(sessionUrl: string): Promise<void> {
  const sessionId = new URL(sessionUrl).pathname.split("/").at(-1);
  if (sessionId === undefined || sessionId.length === 0) return Promise.resolve();
  const cleanupUrl = new URL("/api/v1/media/session-cleanups", window.location.href).toString();
  return new Promise<void>((resolve) => {
    const timer = window.setTimeout(resolve, 2_000);
    void fetch(cleanupUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ schemaVersion: 1, sessionId }),
      keepalive: true
    })
      .catch(() => {
        // Remote cleanup is best-effort and continues independently of recovery.
      })
      .finally(() => {
        window.clearTimeout(timer);
        resolve();
      });
  });
}

async function waitForIceGathering(
  peer: RTCPeerConnection,
  signal: AbortSignal
): Promise<void> {
  if (peer.iceGatheringState === "complete") {
    return;
  }
  await new Promise<void>((resolve, reject) => {
    const cleanup = (): void => {
      peer.removeEventListener("icegatheringstatechange", changed);
      signal.removeEventListener("abort", aborted);
    };
    const changed = (): void => {
      if (peer.iceGatheringState === "complete") {
        cleanup();
        resolve();
      }
    };
    const aborted = (): void => {
      cleanup();
      reject(signal.reason);
    };
    peer.addEventListener("icegatheringstatechange", changed);
    signal.addEventListener("abort", aborted, { once: true });
    if (signal.aborted) aborted();
  });
}

function waitForTrack(peer: RTCPeerConnection, signal: AbortSignal): Promise<MediaStream> {
  return new Promise<MediaStream>((resolve, reject) => {
    const cleanup = (): void => {
      peer.removeEventListener("track", received);
      peer.removeEventListener("connectionstatechange", connectionChanged);
      signal.removeEventListener("abort", aborted);
    };
    const received = (event: RTCTrackEvent): void => {
      cleanup();
      resolve(event.streams[0] ?? new MediaStream([event.track]));
    };
    const connectionChanged = (): void => {
      if (peer.connectionState === "failed") {
        cleanup();
        reject(new Error("WHEP peer failed before delivering a track"));
      }
    };
    const aborted = (): void => {
      cleanup();
      reject(signal.reason);
    };
    peer.addEventListener("track", received);
    peer.addEventListener("connectionstatechange", connectionChanged);
    signal.addEventListener("abort", aborted, { once: true });
    connectionChanged();
    if (signal.aborted) aborted();
  });
}
