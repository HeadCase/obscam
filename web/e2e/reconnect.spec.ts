import { expect, test } from "@playwright/test";

import {
  openLiveViewer,
  restoreAndRelease,
  setExposure,
  takeControl,
  visibleSettings
} from "./support/obs-cam.js";

test("failed WHEP negotiation retries and becomes Live", async ({ page }) => {
  let attempts = 0;
  await page.route("**/obscam/whep", async (route) => {
    attempts += 1;
    if (attempts === 1) {
      await route.abort("connectionfailed");
    } else {
      await route.continue();
    }
  });

  await openLiveViewer(page);

  expect(attempts).toBeGreaterThanOrEqual(2);
});

test("hung WHEP negotiation is bounded and retries", async ({ page }) => {
  let attempts = 0;
  await page.route("**/obscam/whep", async (route) => {
    if (route.request().method() !== "POST") {
      await route.continue();
      return;
    }
    attempts += 1;
    if (attempts === 1) {
      await page.waitForTimeout(12_000);
      try {
        await route.abort("timedout");
      } catch {
        // The browser-side ten-second deadline may already have cancelled it.
      }
    } else {
      await route.continue();
    }
  });

  await page.goto("/");
  await expect(page.locator("[data-service-status]")).toHaveText("Live", {
    timeout: 30_000
  });
  expect(attempts).toBeGreaterThanOrEqual(2);
});

test("hung cleanup after a partial WHEP session cannot block retry", async ({ page }) => {
  let attempts = 0;
  await page.route("**/api/v1/media/session-cleanups", async (route) => {
    await page.waitForTimeout(12_000);
    try {
      await route.abort("timedout");
    } catch {
      // The caller stops waiting after two seconds while cleanup remains best-effort.
    }
  });
  await page.route("**/obscam/whep**", async (route) => {
    const request = route.request();
    if (request.method() === "POST") {
      attempts += 1;
      if (attempts === 1) {
        await route.fulfill({
          status: 201,
          headers: { Location: "/obscam/whep/failed-session" },
          contentType: "application/sdp",
          body: "invalid SDP"
        });
        return;
      }
    }
    await route.continue();
  });

  await openLiveViewer(page);

  expect(attempts).toBeGreaterThanOrEqual(2);
});

test("navigation relays cleanup for the active WHEP session", async ({ page }) => {
  const established = page.waitForResponse((response) =>
    response.request().method() === "POST" && response.url().endsWith("/obscam/whep")
  );
  await openLiveViewer(page);
  const response = await established;
  const location = response.headers().location;
  expect(location).toBeDefined();
  const sessionUrl = new URL(location ?? "", response.url());
  const sessionId = sessionUrl.pathname.split("/").at(-1);
  expect(sessionId).toBeDefined();
  await page.evaluate(() => {
    localStorage.removeItem("obscam.test.sessionCleanup");
    const originalFetch = window.fetch.bind(window);
    window.fetch = (input, init) => {
      if (String(input).endsWith("/api/v1/media/session-cleanups")) {
        localStorage.setItem("obscam.test.sessionCleanup", String(init?.body));
      }
      return originalFetch(input, init);
    };
  });

  await page.goto("/assets/styles.css");

  expect(await page.evaluate(() => localStorage.getItem("obscam.test.sessionCleanup")))
    .toBe(JSON.stringify({ schemaVersion: 1, sessionId }));
});

test("repeated media failures preserve control authority until video recovers", async ({ page }) => {
  let attempts = 0;
  await page.route("**/obscam/whep", async (route) => {
    attempts += 1;
    if (attempts <= 4) {
      await route.abort("connectionfailed");
    } else {
      await route.continue();
    }
  });

  await page.goto("/");
  const original = await visibleSettings(page);
  try {
    await takeControl(page);
    await expect(page.locator("[data-control-status]")).toHaveText("You have control");
    await expect(page.locator("[data-service-status]")).toHaveText("Live");
    await expect(page.locator("[data-control-status]")).toHaveText("You have control");
    expect(attempts).toBeGreaterThanOrEqual(5);
  } finally {
    await restoreAndRelease(page, original);
  }
});

test("control reconnect disables mutation, keeps video Live, and never replays settings", async ({ page }) => {
  const browserSockets: Array<{ close(options?: { code?: number; reason?: string }): Promise<void> }> = [];
  const sentMessages: string[] = [];
  let connections = 0;
  await page.routeWebSocket("**/api/v1/control", (browserSocket) => {
    connections += 1;
    browserSockets.push(browserSocket);
    if (connections === 2) {
      void browserSocket.close({ code: 1012, reason: "test reconnect" });
      return;
    }
    const serverSocket = browserSocket.connectToServer();
    browserSocket.onMessage((message) => {
      const text = message.toString();
      sentMessages.push(text);
      serverSocket.send(message);
    });
  });

  await openLiveViewer(page);
  const original = await visibleSettings(page);
  try {
    await takeControl(page);
    const replacement = original.exposureMs === 20 ? 50 : 20;
    await setExposure(page, replacement);
    const mutationsBeforeReconnect = sentMessages.filter(isSettingsMutation).length;

    await browserSockets[0]?.close({ code: 1012, reason: "test control loss" });
    await expect(page.locator("[data-control-status]")).toContainText("Control reconnecting");
    await expect(page.locator("[data-gain]")).toBeDisabled();
    await expect(page.locator("[data-service-status]")).toHaveText("Live");
    await expect.poll(() => connections).toBeGreaterThanOrEqual(3);
    await expect(page.locator("[data-control-status]")).toHaveText("You have control");
    await page.waitForTimeout(1_000);
    expect(sentMessages.filter(isSettingsMutation)).toHaveLength(mutationsBeforeReconnect);
  } finally {
    await restoreAndRelease(page, original);
  }
});

test("stale same-runtime credentials attempt only lease resume", async ({ page }) => {
  const sentMessages: string[] = [];
  await page.routeWebSocket("**/api/v1/control", (browserSocket) => {
    const serverSocket = browserSocket.connectToServer();
    browserSocket.onMessage((message) => {
      sentMessages.push(message.toString());
      serverSocket.send(message);
    });
  });
  await page.goto("/");
  const runtimeEpoch = await page.evaluate(async () => {
    const response = await fetch("/api/v1/runtime", { cache: "no-store" });
    return (await response.json() as { runtimeEpoch: string }).runtimeEpoch;
  });
  await page.evaluate(({ epoch }) => {
    sessionStorage.setItem("obscam.control.v1", JSON.stringify({
      runtimeEpoch: epoch,
      generation: Number.MAX_SAFE_INTEGER,
      secret: "ab".repeat(32)
    }));
  }, { epoch: runtimeEpoch });
  sentMessages.length = 0;

  await page.reload();
  await expect(page.locator("[data-control-status]")).toHaveText("No one has control");
  await expect.poll(() => sentMessages.some((message) => JSON.parse(message).type === "resume"))
    .toBe(true);
  expect(sentMessages.some(isSettingsMutation)).toBe(false);
  await expect.poll(() => page.evaluate(() => sessionStorage.getItem("obscam.control.v1")))
    .toBeNull();
});

test("foreground return reconnects media and becomes Live again", async ({ page }) => {
  let qualityConnections = 0;
  let controlConnections = 0;
  page.on("websocket", (socket) => {
    if (socket.url().endsWith("/api/v1/control")) controlConnections += 1;
  });
  page.on("response", (response) => {
    if (
      response.request().method() === "POST" &&
      response.url().endsWith("/api/v1/service-quality/connections")
    ) {
      qualityConnections += 1;
    }
  });
  await openLiveViewer(page);
  const status = page.locator("[data-service-status]");
  await status.evaluate((element) => {
    element.setAttribute("data-state-history", element.textContent ?? "");
    new MutationObserver(() => {
      const history = element.getAttribute("data-state-history") ?? "";
      element.setAttribute("data-state-history", `${history},${element.textContent ?? ""}`);
    }).observe(element, { childList: true, characterData: true, subtree: true });
  });
  await page.evaluate(() => {
    document.documentElement.dataset.testVisibilityState = "hidden";
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => document.documentElement.dataset.testVisibilityState
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForTimeout(2_000);

  const retainedPixels = await page.evaluate(() => {
    const canvas = document.querySelector<HTMLCanvasElement>("[data-viewer-retained-frame]");
    const context = canvas?.getContext("2d");
    if (canvas === null || context == null) throw new Error("retained frame is unavailable");
    const pixels = [
      [canvas.width / 4, canvas.height / 4],
      [canvas.width / 2, canvas.height / 2],
      [canvas.width * 3 / 4, canvas.height * 3 / 4]
    ].flatMap(([x, y]) => [...context.getImageData(x ?? 0, y ?? 0, 1, 1).data]);
    document.documentElement.dataset.testVisibilityState = "visible";
    document.dispatchEvent(new Event("visibilitychange"));
    return pixels;
  });
  await expect(status).toHaveText("Reconnecting");
  const retainedFrame = page.locator("[data-viewer-retained-frame]");
  await expect(retainedFrame).toBeVisible();
  expect(await retainedFrame.evaluate((canvas: HTMLCanvasElement) => {
    const context = canvas.getContext("2d");
    if (context === null) throw new Error("retained frame is unavailable");
    return [
      [canvas.width / 4, canvas.height / 4],
      [canvas.width / 2, canvas.height / 2],
      [canvas.width * 3 / 4, canvas.height * 3 / 4]
    ].flatMap(([x, y]) => [...context.getImageData(x ?? 0, y ?? 0, 1, 1).data]);
  })).toEqual(retainedPixels);
  await expect(status).toHaveText("Live");
  await expect(retainedFrame).toBeHidden();
  await expect.poll(() => qualityConnections).toBeGreaterThanOrEqual(2);
  await expect.poll(() => controlConnections).toBeGreaterThanOrEqual(2);
  expect(await status.getAttribute("data-state-history")).toContain("Reconnecting");
  const resumedAt = await page.locator("[data-viewer-video]")
    .evaluate((element: HTMLVideoElement) => element.currentTime);
  await expect.poll(() => page.locator("[data-viewer-video]")
    .evaluate((element: HTMLVideoElement) => element.currentTime))
    .toBeGreaterThan(resumedAt);
});

test("network restoration bypasses delayed transport retries", async ({ page }) => {
  let whepConnections = 0;
  let controlConnections = 0;
  let failedWhepConnections = 0;
  let failedControlConnections = 0;
  await page.route("**/obscam/whep", async (route) => {
    if (route.request().method() === "POST" && failedWhepConnections > 0) {
      failedWhepConnections -= 1;
      await route.abort("connectionfailed");
    } else {
      await route.continue();
    }
  });
  await page.routeWebSocket("**/api/v1/control", (browserSocket) => {
    controlConnections += 1;
    if (failedControlConnections > 0) {
      failedControlConnections -= 1;
      void browserSocket.close({ code: 1012, reason: "test network interruption" });
      return;
    }
    browserSocket.connectToServer();
  });
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().endsWith("/obscam/whep")) {
      whepConnections += 1;
    }
  });
  await openLiveViewer(page);
  const previousWhepConnections = whepConnections;
  failedWhepConnections = 4;
  await page.evaluate(() => window.dispatchEvent(new Event("online")));
  await expect.poll(() => whepConnections).toBeGreaterThanOrEqual(previousWhepConnections + 4);
  await expect(page.locator("[data-service-status]")).toHaveText("Reconnecting");
  await expect.poll(() => failedWhepConnections).toBe(0);
  await page.waitForTimeout(100);
  const mediaConnectionsDuringBackoff = whepConnections;
  await page.waitForTimeout(100);
  expect(whepConnections).toBe(mediaConnectionsDuringBackoff);

  const mediaRestoredAt = Date.now();
  await page.evaluate(() => window.dispatchEvent(new Event("online")));
  await expect.poll(() => whepConnections).toBeGreaterThan(mediaConnectionsDuringBackoff);
  expect(Date.now() - mediaRestoredAt).toBeLessThan(300);
  await expect(page.locator("[data-service-status]")).toHaveText("Live");

  failedControlConnections = 4;
  await page.evaluate(() => window.dispatchEvent(new Event("online")));
  await expect.poll(() => failedControlConnections).toBe(0);
  await expect(page.locator("[data-control-status]")).toContainText("Control reconnecting in");
  await page.waitForTimeout(100);
  const controlConnectionsDuringBackoff = controlConnections;
  await page.waitForTimeout(100);
  expect(controlConnections).toBe(controlConnectionsDuringBackoff);

  const controlRestoredAt = Date.now();
  await page.evaluate(() => window.dispatchEvent(new Event("online")));
  await expect.poll(() => controlConnections).toBeGreaterThan(controlConnectionsDuringBackoff);
  expect(Date.now() - controlRestoredAt).toBeLessThan(300);
  await expect(page.locator("[data-service-status]")).toHaveText("Live");
});

function isSettingsMutation(message: string): boolean {
  return (JSON.parse(message) as { type?: string }).type === "set_settings";
}
