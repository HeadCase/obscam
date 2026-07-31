import { expect, test } from "@playwright/test";

test("default viewing remains Live while native video advances", async ({ page }) => {
  await page.goto("/");

  const status = page.locator("[data-service-status]");
  const video = page.locator("[data-viewer-video]");
  await expect(status).toHaveText("Live");
  await expect.poll(
    () => video.evaluate((element: HTMLVideoElement) => [element.videoWidth, element.videoHeight]),
    { message: "the deployed WHEP track should retain native dimensions" }
  ).toEqual([1920, 1080]);

  const startedAt = await video.evaluate((element: HTMLVideoElement) => element.currentTime);
  const observedStatuses: string[] = [];
  for (let sample = 0; sample < 12; sample += 1) {
    observedStatuses.push((await status.textContent()) ?? "missing");
    await page.waitForTimeout(500);
  }
  const endedAt = await video.evaluate((element: HTMLVideoElement) => element.currentTime);

  expect(observedStatuses, "default viewing should not oscillate away from Live").toEqual(
    Array(12).fill("Live")
  );
  expect(endedAt - startedAt, "video should advance throughout the observation window").toBeGreaterThan(4);
});
