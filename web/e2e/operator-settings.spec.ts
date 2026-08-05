import { expect, test } from "@playwright/test";

import {
  openLiveViewer,
  restoreAndRelease,
  setExposure,
  setGain,
  setTreatment,
  takeControl,
  visibleSettings
} from "./support/obs-cam.js";

test("quality calibration cannot delay the first media request", async ({ page }) => {
  let releaseClock!: () => void;
  let clockStarted!: () => void;
  const clockBlocked = new Promise<void>((resolve) => {
    releaseClock = resolve;
  });
  const sawClock = new Promise<void>((resolve) => {
    clockStarted = resolve;
  });
  await page.route("**/api/v1/clock", async (route) => {
    clockStarted();
    await clockBlocked;
    await route.continue();
  });
  const mediaRequest = page.waitForRequest((request) =>
    request.method() === "POST" && new URL(request.url()).port === "8889"
  );

  try {
    await page.goto("/");
    await sawClock;
    expect(await mediaRequest).toBeTruthy();
  } finally {
    releaseClock();
  }
});

test("operator changes gain without changing exposure or treatment", async ({ page }) => {
  await openLiveViewer(page);
  await takeControl(page);
  const original = await visibleSettings(page);
  const targetGain = original.gain === 150 ? 200 : 150;

  try {
    await setGain(page, targetGain);
    expect(await visibleSettings(page)).toEqual({ ...original, gain: targetGain });
  } finally {
    await restoreAndRelease(page, original);
  }
});

test("operator can discard a multi-setting draft without sending it", async ({ page }) => {
  const sentSettings: string[] = [];
  page.on("websocket", (socket) => socket.on("framesent", ({ payload }) => {
    if (typeof payload === "string" && payload.includes('\"type\":\"set_settings\"')) {
      sentSettings.push(payload);
    }
  }));
  await openLiveViewer(page);
  await takeControl(page);
  const original = await visibleSettings(page);
  const targetExposure = original.exposureMs === 300 ? 500 : 300;
  const targetTreatment = original.treatment === "monochrome" ? "colour" : "monochrome";

  await page.locator(`[data-exposure-ms="${targetExposure}"]`).click();
  await page.locator(`[data-control="treatment-${targetTreatment}"]`).click();
  await expect(page.locator("[data-control-status]")).toHaveText("Unsaved changes");
  await expect(page.getByRole("button", { name: "Apply" })).toBeEnabled();
  await page.getByRole("button", { name: "Discard" }).click();

  expect(await visibleSettings(page)).toEqual(original);
  await expect(page.locator("[data-control-status]")).toHaveText("You have control");
  expect(sentSettings).toEqual([]);
  await page.getByRole("button", { name: "Release control" }).click();
});

test("operator changes exposure without changing gain or treatment", async ({ page }) => {
  await openLiveViewer(page);
  await takeControl(page);
  const original = await visibleSettings(page);
  const targetExposure = original.exposureMs === 300 ? 500 : 300;

  try {
    await setExposure(page, targetExposure);
    expect(await visibleSettings(page)).toEqual({ ...original, exposureMs: targetExposure });
  } finally {
    await restoreAndRelease(page, original);
  }
});

test("operator changes treatment without changing exposure or gain", async ({ page }) => {
  await openLiveViewer(page);
  await takeControl(page);
  const original = await visibleSettings(page);
  const targetTreatment = original.treatment === "monochrome" ? "colour" : "monochrome";

  try {
    await setTreatment(page, targetTreatment);
    expect(await visibleSettings(page)).toEqual({ ...original, treatment: targetTreatment });
  } finally {
    await restoreAndRelease(page, original);
  }
});

test("four viewers keep presenting while one viewer applies settings", async ({ browser }) => {
  const contexts = await Promise.all(Array.from({ length: 4 }, () => browser.newContext()));
  const pages = await Promise.all(contexts.map((context) => context.newPage()));
  const operator = pages[0];
  if (operator === undefined) throw new Error("four-viewer test needs an operator page");
  try {
    await Promise.all(pages.map((page) => openLiveViewer(page)));
    await takeControl(operator);
    const original = await visibleSettings(operator);
    const targetExposure = original.exposureMs === 300 ? 500 : 300;
    const started = await Promise.all(pages.map((page) =>
      page.locator("[data-viewer-video]").evaluate((video: HTMLVideoElement) => video.currentTime)
    ));

    await setExposure(operator, targetExposure);
    await operator.waitForTimeout(1_500);
    const ended = await Promise.all(pages.map((page) =>
      page.locator("[data-viewer-video]").evaluate((video: HTMLVideoElement) => video.currentTime)
    ));

    for (let index = 0; index < pages.length; index += 1) {
      expect(ended[index]!).toBeGreaterThan(started[index]!);
    }
    await restoreAndRelease(operator, original);
  } finally {
    await Promise.all(contexts.map((context) => context.close()));
  }
});

test("operator moves from short to long exposure and back without losing the retained video", async ({ page }) => {
  test.setTimeout(90_000);
  await openLiveViewer(page);
  await takeControl(page);
  const original = await visibleSettings(page);

  try {
    await setExposure(page, 100);
    const shortSettings = await visibleSettings(page);
    expect(shortSettings).toEqual({ ...original, exposureMs: 100 });

    const video = page.locator("[data-viewer-video]");
    const startedAt = await video.evaluate((element: HTMLVideoElement) => element.currentTime);
    const longExposure = page.locator("[data-exposure-ms=\"30000\"]");
    await longExposure.click();
    await expect(longExposure).toHaveAttribute("aria-pressed", "true");
    await page.getByRole("button", { name: "Apply" }).click();
    await expect(page.locator("[data-service-detail]")).toContainText("Applying generation");
    await page.waitForTimeout(3_000);
    await expect(page.locator("[data-viewer-status]")).toHaveText("Live");
    await expect(page.locator("[data-service-detail]")).toContainText("Applying generation");
    const retainedAt = await video.evaluate((element: HTMLVideoElement) => element.currentTime);
    expect(retainedAt - startedAt, "retained video should keep playing during a long exposure")
      .toBeGreaterThan(2);

    await setExposure(page, 100);
    expect(await visibleSettings(page)).toEqual(shortSettings);
  } finally {
    await restoreAndRelease(page, original);
  }
});
