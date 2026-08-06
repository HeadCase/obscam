import { expect, test } from "@playwright/test";

import { openLiveViewer, takeControl } from "./support/obs-cam.js";

test("releasing authority never presents a settings Applying rail", async ({ page }) => {
  await openLiveViewer(page);
  await takeControl(page);
  await page.evaluate(() => {
    const rail = document.querySelector<HTMLElement>("[data-exposure-rail]")!;
    const classes: string[] = [];
    new MutationObserver(() => classes.push(rail.className)).observe(rail, {
      attributes: true,
      attributeFilter: ["class"]
    });
    (window as Window & { __releaseRailClasses?: string[] }).__releaseRailClasses = classes;
  });

  await page.getByRole("button", { name: "Release", exact: true }).click();
  await expect(page.getByRole("button", { name: "Take control" })).toBeVisible();
  expect(await page.evaluate(() =>
    ((window as Window & { __releaseRailClasses?: string[] }).__releaseRailClasses ?? [])
      .some((value) => value.includes("is-applying"))
  )).toBe(false);
});

test("a second viewer can observe and explicitly take authority", async ({ context, page }) => {
  await openLiveViewer(page);
  await takeControl(page);
  const second = await context.newPage();

  try {
    await openLiveViewer(second);
    await expect(second.locator("[data-gain]")).toBeDisabled();
    await expect(second.locator("[data-exposure]")).toBeDisabled();
    await expect(second.getByRole("button", { name: "Settings" })).toHaveAttribute(
      "title",
      "Take control to change settings"
    );

    await second.getByRole("button", { name: "Take control" }).click();
    await expect(second.getByRole("button", { name: "Release", exact: true })).toBeVisible();
    await expect(second.locator("[data-settings-popover]")).toBeVisible();
    await expect(page.locator("[data-announcement]")).toHaveText("Control taken by another viewer");
    await expect(page.locator("[data-gain]")).toBeDisabled();
  } finally {
    const release = second.getByRole("button", { name: "Release", exact: true });
    if (await release.isVisible()) {
      await release.click();
    }
  }
});
