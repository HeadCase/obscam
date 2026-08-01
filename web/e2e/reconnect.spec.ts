import { expect, test } from "@playwright/test";

import { openLiveViewer } from "./support/obs-cam.js";

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

test("foreground return reconnects media and becomes Live again", async ({ page }) => {
  let qualityConnections = 0;
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

  await page.evaluate(() => {
    document.documentElement.dataset.testVisibilityState = "visible";
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect(status).toHaveText("Live");
  await expect.poll(() => qualityConnections).toBeGreaterThanOrEqual(2);
  expect(await status.getAttribute("data-state-history")).toContain("Reconnecting");
  const resumedAt = await page.locator("[data-viewer-video]")
    .evaluate((element: HTMLVideoElement) => element.currentTime);
  await expect.poll(() => page.locator("[data-viewer-video]")
    .evaluate((element: HTMLVideoElement) => element.currentTime))
    .toBeGreaterThan(resumedAt);
});
