import { expect, test } from "@playwright/test";

import { openLiveViewer, takeControl } from "./support/obs-cam.js";

test("a second viewer can observe and explicitly take authority", async ({ context, page }) => {
  await openLiveViewer(page);
  await takeControl(page);
  const second = await context.newPage();

  try {
    await openLiveViewer(second);
    await expect(second.locator("[data-control-status]")).toHaveText("Another viewer has control");
    await expect(second.locator("[data-gain]")).toBeDisabled();
    await expect(second.locator("[data-exposure-ms=\"500\"]")).toBeDisabled();

    await second.getByRole("button", { name: "Take control" }).click();
    await expect(second.locator("[data-control-status]")).toHaveText("You have control");
    await expect(page.locator("[data-control-status]")).toHaveText("Another viewer has control");
    await expect(page.locator("[data-gain]")).toBeDisabled();
  } finally {
    const release = second.getByRole("button", { name: "Release control" });
    if (await release.isVisible()) {
      await release.click();
    }
  }
});
