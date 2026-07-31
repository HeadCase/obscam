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
    await expect(page.locator("[data-service-status]")).toHaveText("Capturing");
    await page.waitForTimeout(3_000);
    await expect(page.locator("[data-service-status]")).toHaveText("Capturing");
    const retainedAt = await video.evaluate((element: HTMLVideoElement) => element.currentTime);
    expect(retainedAt - startedAt, "retained video should keep playing during a long exposure")
      .toBeGreaterThan(2);

    await setExposure(page, 100);
    expect(await visibleSettings(page)).toEqual(shortSettings);
  } finally {
    await restoreAndRelease(page, original);
  }
});
