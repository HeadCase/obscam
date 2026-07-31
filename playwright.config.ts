import { defineConfig, devices } from "@playwright/test";

const baseURL = process.env.OBSCAM_BASE_URL ?? "http://10.164.190.1:8080";

export default defineConfig({
  testDir: "web/e2e",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 60_000,
  expect: { timeout: 20_000 },
  outputDir: "test-results",
  reporter: [["line"], ["html", { open: "never", outputFolder: "playwright-report" }]],
  use: {
    baseURL,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: "retain-on-failure"
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] }
    }
  ]
});
