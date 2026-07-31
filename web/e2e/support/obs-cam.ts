import { expect, type Page } from "@playwright/test";

export interface VisibleSettings {
  exposureMs: number;
  gain: number;
  treatment: "monochrome" | "colour";
}

export async function openLiveViewer(page: Page): Promise<void> {
  await page.goto("/");
  await expect(page.locator("[data-service-status]")).toHaveText("Live");
}

export async function takeControl(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Take control" }).click();
  await expect(page.locator("[data-control-status]")).toHaveText("You have control");
}

export async function visibleSettings(page: Page): Promise<VisibleSettings> {
  const exposure = page.locator("[data-exposure-ms][aria-pressed=\"true\"]");
  const treatment = page.locator("[data-control^=\"treatment-\"][aria-pressed=\"true\"]");
  return {
    exposureMs: Number(await exposure.getAttribute("data-exposure-ms")),
    gain: Number(await page.locator("[data-gain]").inputValue()),
    treatment: (await treatment.getAttribute("data-control")) === "treatment-colour"
      ? "colour"
      : "monochrome"
  };
}

export async function setGain(page: Page, gain: number): Promise<void> {
  await page.locator("[data-gain]").fill(String(gain));
  await expect(page.locator("[data-gain-output]")).toHaveText(String(gain));
  await waitUntilVisible(page);
}

export async function setExposure(page: Page, exposureMs: number): Promise<void> {
  const button = page.locator(`[data-exposure-ms="${exposureMs}"]`);
  await button.click();
  await expect(button).toHaveAttribute("aria-pressed", "true");
  await waitUntilVisible(page);
}

export async function setTreatment(
  page: Page,
  treatment: VisibleSettings["treatment"]
): Promise<void> {
  const button = page.locator(`[data-control="treatment-${treatment}"]`);
  await button.click();
  await expect(button).toHaveAttribute("aria-pressed", "true");
  await waitUntilVisible(page);
}

export async function restoreAndRelease(
  page: Page,
  settings: VisibleSettings
): Promise<void> {
  const release = page.getByRole("button", { name: "Release control" });
  if (!(await release.isVisible())) return;
  const current = await visibleSettings(page);
  if (current.exposureMs !== settings.exposureMs) {
    await setExposure(page, settings.exposureMs);
  }
  if (current.gain !== settings.gain) {
    await setGain(page, settings.gain);
  }
  if (current.treatment !== settings.treatment) {
    await setTreatment(page, settings.treatment);
  }
  await release.click();
}

async function waitUntilVisible(page: Page): Promise<void> {
  await expect(page.locator("[data-control-status]")).toHaveText("You have control");
  await expect(page.locator("[data-service-status]")).toHaveText("Live");
}
