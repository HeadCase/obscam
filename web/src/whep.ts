/** One direct browser-to-MediaMTX WHEP session. */
export interface WhepSession {
  close(): Promise<void>;
}

/** Negotiates a receive-only WebRTC video track through WHEP. */
export async function startWhep(
  video: HTMLVideoElement,
  endpoint: string,
  onFailure: () => void
): Promise<WhepSession> {
  const peer = new RTCPeerConnection();
  peer.addTransceiver("video", { direction: "recvonly" });
  peer.addEventListener("track", (event) => {
    video.srcObject = event.streams[0] ?? new MediaStream([event.track]);
  });

  try {
    const offer = await peer.createOffer();
    await peer.setLocalDescription(offer);
    await waitForIceGathering(peer);
    const localDescription = peer.localDescription;
    if (localDescription === null) {
      throw new Error("WHEP offer was not established");
    }

    const response = await fetch(endpoint, {
      method: "POST",
      headers: { "Content-Type": "application/sdp" },
      body: localDescription.sdp
    });
    if (response.status !== 201) {
      throw new Error(`WHEP request failed with ${response.status}`);
    }
    const resource = response.headers.get("Location");
    if (resource === null) {
      throw new Error("WHEP response omitted its session resource");
    }
    const sessionUrl = new URL(resource, endpoint).toString();
    await peer.setRemoteDescription({ type: "answer", sdp: await response.text() });
    let closed = false;
    let failureReported = false;
    peer.addEventListener("connectionstatechange", () => {
      if (!closed && !failureReported && peer.connectionState === "failed") {
        failureReported = true;
        onFailure();
      }
    });

    return {
      async close(): Promise<void> {
        closed = true;
        peer.close();
        try {
          await fetch(sessionUrl, { method: "DELETE" });
        } catch {
          // The peer is already closed; remote cleanup is best-effort.
        }
      }
    };
  } catch (error: unknown) {
    peer.close();
    throw error;
  }
}

async function waitForIceGathering(peer: RTCPeerConnection): Promise<void> {
  if (peer.iceGatheringState === "complete") {
    return;
  }
  await new Promise<void>((resolve) => {
    const changed = (): void => {
      if (peer.iceGatheringState === "complete") {
        peer.removeEventListener("icegatheringstatechange", changed);
        resolve();
      }
    };
    peer.addEventListener("icegatheringstatechange", changed);
  });
}
