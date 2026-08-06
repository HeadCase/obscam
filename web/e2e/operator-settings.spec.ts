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

async function setExposureWithinExactDeadline(
  page: import("@playwright/test").Page,
  exposureMs: number,
  hardDeadlineMs: number
): Promise<number> {
  const appliedMarks = await page.evaluate(() =>
    performance.getEntriesByName("obscam.settings.camera-applied").length
  );
  const startedAt = await page.evaluate(() => performance.now());
  const choices = [50, 100, 200, 300, 500, 1_000, 2_000, 5_000, 10_000, 15_000, 20_000, 30_000];
  const slider = page.locator("[data-exposure]");
  await slider.fill(String(choices.indexOf(exposureMs)));
  await expect(slider).toHaveAttribute("data-exposure-ms", String(exposureMs));
  await page.getByRole("button", { name: "Apply" }).click();
  const targetGeneration = await page.waitForFunction((priorCount) => {
    const marks = performance.getEntriesByName("obscam.settings.camera-applied") as PerformanceMark[];
    if (marks.length <= priorCount) return null;
    const detail = marks.at(-1)?.detail as { settingsGeneration?: unknown } | undefined;
    return typeof detail?.settingsGeneration === "number" ? detail.settingsGeneration : null;
  }, appliedMarks, { timeout: hardDeadlineMs }).then((handle) => handle.jsonValue());
  if (typeof targetGeneration !== "number") throw new Error("camera-applied mark had no generation");
  await page.waitForFunction((generation) =>
    (performance.getEntriesByName("obscam.settings.browser-presented-exact") as PerformanceMark[])
      .some((mark) => {
        const detail = mark.detail as { settingsGeneration?: unknown } | undefined;
        return detail?.settingsGeneration === generation;
      }), targetGeneration, { timeout: hardDeadlineMs });
  const elapsedMs = await page.evaluate((start) => performance.now() - start, startedAt);
  expect(elapsedMs).toBeLessThanOrEqual(hardDeadlineMs);
  return elapsedMs;
}

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

  const exposureChoices = [50, 100, 200, 300, 500, 1_000, 2_000, 5_000, 10_000, 15_000, 20_000, 30_000];
  await page.locator("[data-exposure]").fill(String(exposureChoices.indexOf(targetExposure)));
  await expect(page.locator('[data-setting-semantics="exposure"]')).toContainText(
    targetExposure >= 1_000 ? `Draft ${targetExposure / 1_000} s` : `Draft ${targetExposure} ms`
  );
  await page.locator(`[data-control="treatment-${targetTreatment}"]`).click();
  await expect(page.locator('[data-setting-state="draft"]')).toHaveCount(2);
  await expect(page.getByRole("button", { name: "Apply" })).toBeEnabled();
  await page.getByRole("button", { name: "Discard" }).click();

  expect(await visibleSettings(page)).toEqual(original);
  await expect(page.locator('[data-setting-state]:not([data-setting-state="visible"])')).toHaveCount(0);
  expect(sentSettings).toEqual([]);
  await page.getByRole("button", { name: "Release", exact: true }).click();
});

test("Enter on a native settings button never submits a pre-existing draft", async ({ page }) => {
  const sentSettings: string[] = [];
  page.on("websocket", (socket) => socket.on("framesent", ({ payload }) => {
    if (typeof payload === "string" && payload.includes('\"type\":\"set_settings\"')) {
      sentSettings.push(payload);
    }
  }));
  await openLiveViewer(page);
  await takeControl(page);
  const original = await visibleSettings(page);
  const choices = [50, 100, 200, 300, 500, 1_000, 2_000, 5_000, 10_000, 15_000, 20_000, 30_000];
  const targetExposure = original.exposureMs === 300 ? 500 : 300;
  const targetTreatment = original.treatment === "monochrome" ? "colour" : "monochrome";

  await page.locator("[data-exposure]").fill(String(choices.indexOf(targetExposure)));
  await page.locator(`[data-control="treatment-${targetTreatment}"]`).focus();
  await page.keyboard.press("Enter");
  expect(sentSettings).toEqual([]);
  await page.getByRole("button", { name: "Discard" }).click();
  await page.getByRole("button", { name: "Release", exact: true }).click();
});

test("hiding and reopening settings preserves an unapplied draft", async ({ page }) => {
  await openLiveViewer(page);
  await takeControl(page);
  const original = await visibleSettings(page);
  const choices = [50, 100, 200, 300, 500, 1_000, 2_000, 5_000, 10_000, 15_000, 20_000, 30_000];
  const target = original.exposureMs === 300 ? 500 : 300;

  await page.locator("[data-exposure]").fill(String(choices.indexOf(target)));
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.locator("[data-settings-popover]")).toBeHidden();
  await expect(page.getByRole("button", { name: "Settings" })).toHaveClass(/has-draft/u);
  await expect(page.getByRole("button", { name: "Settings" })).toHaveAttribute(
    "aria-description",
    "Unapplied changes"
  );
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.locator("[data-exposure]")).toHaveAttribute("data-exposure-ms", String(target));
  await page.getByRole("button", { name: "Discard" }).click();
  await page.getByRole("button", { name: "Release", exact: true }).click();
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

test("authoritative exposure rail advances smoothly without mid-capture rewinds", async ({ page }) => {
  await openLiveViewer(page);
  await takeControl(page);
  const original = await visibleSettings(page);

  try {
    await page.evaluate(() => {
      const rail = document.querySelector<HTMLElement>("[data-exposure-rail]")!;
      const classes = new Set<string>();
      const record = (): void => {
        for (const value of Array.from(rail.classList)) classes.add(value);
      };
      new MutationObserver(record).observe(rail, {
        attributes: true,
        attributeFilter: ["class"]
      });
      record();
      (window as Window & { __observedRailClasses?: Set<string> }).__observedRailClasses = classes;
    });
    await setExposure(page, 1_000);
    expect(await page.evaluate(() => [
      ...((window as Window & { __observedRailClasses?: Set<string> }).__observedRailClasses ?? [])
    ])).toContain("is-applying");
    const samples = await page.evaluate(async () => {
      const fill = document.querySelector<HTMLElement>("[data-exposure-rail-fill]")!;
      const rail = document.querySelector<HTMLElement>("[data-exposure-rail]")!;
      const values: number[] = [];
      const startedAt = performance.now();
      await new Promise<void>((resolve) => {
        const sample = (): void => {
          values.push(rail.hidden ? -1 : Number.parseFloat(fill.style.width || "0") / 100);
          if (performance.now() - startedAt >= 2_600) resolve();
          else requestAnimationFrame(sample);
        };
        requestAnimationFrame(sample);
      });
      return values;
    });
    const visible = samples.filter((value) => value >= 0);
    expect(visible.length).toBeGreaterThan(100);
    expect(new Set(visible.map((value) => value.toFixed(4))).size).toBeGreaterThan(100);
    for (let index = 1; index < visible.length; index += 1) {
      const prior = visible[index - 1]!;
      const current = visible[index]!;
      if (prior < 0.8) {
        expect(current, `rail rewound from ${prior} to ${current}`).toBeGreaterThanOrEqual(
          prior - 0.02
        );
      }
    }
  } finally {
    await restoreAndRelease(page, original);
  }
});

test("reduced motion retains rail position without continuous interpolation", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openLiveViewer(page);
  await takeControl(page);
  const original = await visibleSettings(page);

  try {
    await setExposure(page, 1_000);
    const widths = await page.evaluate(async () => {
      const fill = document.querySelector<HTMLElement>("[data-exposure-rail-fill]")!;
      const values: string[] = [];
      const startedAt = performance.now();
      await new Promise<void>((resolve) => {
        const sample = (): void => {
          values.push(fill.style.width);
          if (performance.now() - startedAt >= 600) resolve();
          else requestAnimationFrame(sample);
        };
        requestAnimationFrame(sample);
      });
      return values;
    });
    expect(new Set(widths).size).toBeLessThanOrEqual(4);
  } finally {
    await restoreAndRelease(page, original);
  }
});

test("five-second exposure returns exactly visible at 200 ms within the hard SLA", async ({ page }) => {
  test.setTimeout(30_000);
  await openLiveViewer(page);
  await takeControl(page);
  const original = await visibleSettings(page);

  try {
    await setExposure(page, 5_000);
    await setExposureWithinExactDeadline(page, 200, 1_200);
    expect(await visibleSettings(page)).toEqual({ ...original, exposureMs: 200 });
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
    const longExposure = page.locator("[data-exposure]");
    await longExposure.fill("11");
    await expect(longExposure).toHaveAttribute("data-exposure-ms", "30000");
    await page.getByRole("button", { name: "Apply" }).click();
    await expect(page.locator("[data-service-detail]")).toContainText("Exposure in progress");
    await page.waitForTimeout(3_000);
    await expect(page.locator("[data-viewer-status]")).toHaveText("Live");
    await expect(page.locator("[data-service-detail]")).toContainText("Exposure in progress");
    const retainedAt = await video.evaluate((element: HTMLVideoElement) => element.currentTime);
    expect(retainedAt - startedAt, "retained video should keep playing during a long exposure")
      .toBeGreaterThan(2);

    await page.evaluate(() => {
      const container = document.querySelector<HTMLElement>('[data-setting-slider="exposure"]');
      const observed = new Set<string>();
      const record = (): void => {
        if (container?.dataset.settingState !== undefined) {
          observed.add(container.dataset.settingState);
        }
      };
      new MutationObserver(record).observe(container!, {
        attributes: true,
        attributeFilter: ["data-setting-state"]
      });
      record();
      (window as Window & { __observedSettingStates?: Set<string> }).__observedSettingStates = observed;
    });
    await setExposureWithinExactDeadline(page, 100, 1_100);
    const observedStates = await page.evaluate(() => [
      ...((window as Window & { __observedSettingStates?: Set<string> }).__observedSettingStates ?? [])
    ]);
    expect(observedStates).toEqual(expect.arrayContaining(["requested", "applied", "visible"]));
    await expect(page.locator('[data-setting-slider="exposure"]')).toHaveAttribute(
      "data-setting-state",
      "visible"
    );
    expect(await visibleSettings(page)).toEqual(shortSettings);
  } finally {
    await restoreAndRelease(page, original);
  }
});
