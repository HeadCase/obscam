import { readFile } from "node:fs/promises";

import { expect, test } from "@playwright/test";

import { openLiveViewer } from "./support/obs-cam.js";

test("controls retain the approved prototype geometry and slider treatment", async ({ page }) => {
  await page.setViewportSize({ width: 1512, height: 982 });
  await openLiveViewer(page);
  await page.getByRole("button", { name: "Take control" }).click();
  await expect(page.locator("[data-settings-popover]")).toBeVisible();

  await expect(page.locator('[data-setting-slider="exposure"] .slider-ticks > i')).toHaveCount(12);
  await expect(page.locator('[data-setting-slider="gain"] .slider-ticks > i')).toHaveCount(13);
  expect(await page.locator("[data-exposure], [data-gain]").evaluateAll((sliders) =>
    sliders.map((slider) => getComputedStyle(slider).appearance)
  )).toEqual(["none", "none"]);

  const geometry = await page.evaluate(() => {
    const bar = document.querySelector<HTMLElement>("[data-command-bar]")!.getBoundingClientRect();
    const popover = document.querySelector<HTMLElement>("[data-settings-popover]")!
      .getBoundingClientRect();
    return {
      centreDelta: Math.abs(bar.x + bar.width / 2 - (popover.x + popover.width / 2)),
      attachmentGap: bar.y - popover.bottom,
      width: bar.width
    };
  });
  expect(geometry.centreDelta).toBeLessThanOrEqual(1);
  expect(geometry.attachmentGap).toBeGreaterThanOrEqual(4);
  expect(geometry.attachmentGap).toBeLessThanOrEqual(8);

  const exposure = page.locator("[data-exposure]");
  const initialValue = await exposure.inputValue();
  const sliderBox = await exposure.boundingBox();
  if (sliderBox === null) throw new Error("exposure slider has no box");
  await page.mouse.move(sliderBox.x + sliderBox.width * 0.4, sliderBox.y + sliderBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(sliderBox.x + sliderBox.width * 0.85, sliderBox.y + sliderBox.height / 2, {
    steps: 12
  });
  await page.mouse.up();
  expect(await exposure.inputValue()).not.toBe(initialValue);
  const changedWidth = await page.locator("[data-command-bar]").evaluate((bar) =>
    bar.getBoundingClientRect().width
  );
  expect(Math.abs(changedWidth - geometry.width)).toBeLessThanOrEqual(1);
  await page.getByRole("button", { name: "Discard" }).click();
  await page.getByRole("button", { name: "Release", exact: true }).click();
});

test("viewer has no browser errors or failed application requests", async ({ page }) => {
  const consoleErrors: string[] = [];
  const pageErrors: string[] = [];
  const failedResponses: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
  page.on("pageerror", (error) => pageErrors.push(error.message));
  page.on("response", (response) => {
    if (response.status() >= 400) {
      failedResponses.push(
        `${response.request().method()} ${response.status()} ${response.url()}`
      );
    }
  });

  await openLiveViewer(page);
  await page.waitForTimeout(5_000);

  expect(consoleErrors).toEqual([]);
  expect(pageErrors).toEqual([]);
  expect(failedResponses).toEqual([]);
});

test("approved viewports preserve source geometry and avoid horizontal overflow", async ({ page }) => {
  await page.setViewportSize({ width: 430, height: 932 });
  await openLiveViewer(page);
  const source = await page.locator("[data-viewer-video]").evaluate((video: HTMLVideoElement) => ({
    width: video.videoWidth,
    height: video.videoHeight,
    fit: getComputedStyle(video).objectFit
  }));
  expect(source).toEqual({ width: 1920, height: 1080, fit: "contain" });
  await expect(page.locator("[data-exposure]")).toHaveAttribute("min", "0");
  await expect(page.locator("[data-exposure]")).toHaveAttribute("max", "11");
  await expect(page.locator("[data-exposure]")).not.toHaveAttribute("data-exposure-ms", "20");

  for (const viewport of [
    { width: 430, height: 932 },
    { width: 932, height: 430 },
    { width: 756, height: 490 },
    { width: 1512, height: 982 }
  ]) {
    await page.setViewportSize(viewport);
    await page.mouse.move(1, 1);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(viewport.width);
    await expect(page.locator("[data-service-status]")).toBeVisible();
    await expect(page.locator("[data-control=\"take-control\"]")).toBeVisible();
    expect(await page.locator("[data-viewer-video]").boundingBox()).toEqual({
      x: 0,
      y: 0,
      width: viewport.width,
      height: viewport.height
    });
  }
});

test("chrome hides after genuine inactivity and protected interaction keeps it visible", async ({ page }) => {
  test.setTimeout(25_000);
  await openLiveViewer(page);
  const viewer = page.locator("[data-viewer]");
  await expect(viewer).toHaveAttribute("data-chrome-visible", "true");
  await page.waitForTimeout(5_100);
  await expect(viewer).toHaveAttribute("data-chrome-visible", "false");

  await page.mouse.move(1, 1);
  await expect(viewer).toHaveAttribute("data-chrome-visible", "true");
  await page.getByRole("button", { name: "Take control" }).focus();
  await page.waitForTimeout(5_100);
  await expect(viewer).toHaveAttribute("data-chrome-visible", "true");
  await page.getByRole("button", { name: "Take control" }).blur();
  await page.waitForTimeout(5_100);
  await expect(viewer).toHaveAttribute("data-chrome-visible", "false");
  await page.locator("[data-viewer-video]").dispatchEvent("pointerdown");
  await expect(viewer).toHaveAttribute("data-chrome-visible", "true");
  await page.locator("[data-viewer-video]").dispatchEvent("pointerdown");
  await expect(viewer).toHaveAttribute("data-chrome-visible", "false");
});

test("snapshot fallback downloads only native-dimension visible pixels", async ({ page }) => {
  await openLiveViewer(page);
  await expect(page.locator('[data-setting-slider="exposure"]')).toHaveAttribute(
    "data-setting-state",
    "visible"
  );
  const dimensions = await page.locator("[data-viewer-video]").evaluate((video: HTMLVideoElement) => ({
    width: video.videoWidth,
    height: video.videoHeight
  }));
  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Snapshot" }).click();
  const saved = await download;
  expect(saved.suggestedFilename()).toMatch(
    /^obscam-captured-\d{8}T\d{6}\.\d{3}Z-generation-\d+\.png$/u
  );
  const path = await saved.path();
  if (path === null) throw new Error("snapshot download has no local path");
  const bytes = await readFile(path);
  expect(bytes.subarray(1, 4).toString("ascii")).toBe("PNG");
  expect(bytes.readUInt32BE(16)).toBe(dimensions.width);
  expect(bytes.readUInt32BE(20)).toBe(dimensions.height);
});

test("snapshot uses Save As where the browser exposes it", async ({ page }) => {
  await openLiveViewer(page);
  await expect(page.locator('[data-setting-slider="exposure"]')).toHaveAttribute(
    "data-setting-state",
    "visible"
  );
  await page.evaluate(() => {
    Object.defineProperty(window, "isSecureContext", { configurable: true, value: true });
    (window as Window & {
      __savedSnapshot?: { name: string; size: number };
      showSaveFilePicker?: (options: { suggestedName: string }) => Promise<unknown>;
    }).showSaveFilePicker =
      async ({ suggestedName }: { suggestedName: string }) => ({
        createWritable: async () => ({
          write: async (blob: Blob) => {
            (window as Window & { __savedSnapshot?: { name: string; size: number } })
              .__savedSnapshot = { name: suggestedName, size: blob.size };
          },
          close: async () => {}
        })
      });
  });

  await page.getByRole("button", { name: "Snapshot" }).click();
  await expect(page.locator("[data-announcement]")).toHaveText("Snapshot saved");
  const saved = await page.evaluate(() =>
    (window as Window & { __savedSnapshot?: { name: string; size: number } }).__savedSnapshot
  );
  expect(saved?.name).toMatch(/^obscam-captured-.+\.png$/u);
  expect(saved?.size).toBeGreaterThan(0);
});
