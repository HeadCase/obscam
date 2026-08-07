import { expect, type Page } from "@playwright/test";

import { EXPOSURE_CHOICES_MS } from "../../src/control.js";

export interface VisibleSettings {
  exposureMs: number;
  gain: number;
  treatment: "monochrome" | "colour";
}

export function exposureIndex(exposureMs: number): number {
  const index = EXPOSURE_CHOICES_MS.findIndex((choice) => choice === exposureMs);
  if (index < 0) throw new Error(`unsupported exposure ${exposureMs}`);
  return index;
}

export async function openLiveViewer(page: Page): Promise<void> {
  await page.goto("/");
  await expect(page.locator("[data-service-status]")).toHaveText("Live");
}

export async function takeControl(page: Page): Promise<void> {
  await page.mouse.move(1, 1);
  await page.getByRole("button", { name: "Take control" }).click();
  await expect(page.getByRole("button", { name: "Release", exact: true })).toBeVisible();
  await expect(page.locator("[data-settings-popover]")).toBeVisible();
}

export async function visibleSettings(page: Page): Promise<VisibleSettings> {
  const exposure = page.locator("[data-exposure]");
  const treatment = page.locator("[data-control^=\"treatment-\"][aria-pressed=\"true\"]");
  return {
    exposureMs: Number(await exposure.getAttribute("data-exposure-ms")),
    gain: Number(await page.locator("[data-gain-output]").textContent()),
    treatment: (await treatment.getAttribute("data-control")) === "treatment-colour"
      ? "colour"
      : "monochrome"
  };
}

export async function setGain(page: Page, gain: number): Promise<void> {
  await ensureSettingsOpen(page);
  await page.locator("[data-gain]").fill(String(gain / 50));
  await expect(page.locator("[data-gain-output]")).toHaveText(String(gain));
  await page.getByRole("button", { name: "Apply" }).click();
  await waitUntilVisible(page);
}

export async function setExposure(page: Page, exposureMs: number): Promise<void> {
  await ensureSettingsOpen(page);
  const slider = page.locator("[data-exposure]");
  await slider.fill(String(exposureIndex(exposureMs)));
  await expect(slider).toHaveAttribute("data-exposure-ms", String(exposureMs));
  await page.getByRole("button", { name: "Apply" }).click();
  await waitUntilVisible(page);
}

export async function setTreatment(
  page: Page,
  treatment: VisibleSettings["treatment"]
): Promise<void> {
  await ensureSettingsOpen(page);
  const button = page.locator(`[data-control="treatment-${treatment}"]`);
  await button.click();
  await expect(button).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Apply" }).click();
  await waitUntilVisible(page);
}

export async function restoreAndRelease(
  page: Page,
  settings: VisibleSettings
): Promise<void> {
  await page.mouse.move(1, 1);
  const release = page.getByRole("button", { name: "Release", exact: true });
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

async function ensureSettingsOpen(page: Page): Promise<void> {
  await page.mouse.move(1, 1);
  if (await page.locator("[data-settings-popover]").isHidden()) {
    await page.getByRole("button", { name: "Settings" }).click();
  }
}

async function waitUntilVisible(page: Page): Promise<void> {
  await expect(page.locator('[data-setting-state]:not([data-setting-state="visible"])')).toHaveCount(0);
  await expect(page.locator("[data-service-status]")).toHaveText("Live");
}
