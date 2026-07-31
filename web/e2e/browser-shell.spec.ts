import { expect, test } from "@playwright/test";

import { openLiveViewer } from "./support/obs-cam.js";

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

test("operator controls remain reachable without horizontal overflow", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await openLiveViewer(page);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await expect(page.locator("[data-exposure-ms]")).toHaveCount(14);
  await page.locator("[data-control=\"take-control\"]").scrollIntoViewIfNeeded();
  await expect(page.locator("[data-control=\"take-control\"]")).toBeVisible();

  await page.setViewportSize({ width: 1440, height: 900 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(1440);
  await expect(page.locator("[data-service-status]")).toBeVisible();
  await expect(page.locator("[data-control=\"take-control\"]")).toBeVisible();
});
