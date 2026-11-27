// End-to-end tests (e2e/): the built site (`npm run build` first) in Chromium,
// WebKit and Firefox, played through the page as a person would.
import { defineConfig, devices } from "@playwright/test";

const PORT = 4329;

export default defineConfig({
  testDir: "e2e",
  // Whole games against the engine take a while: it waits 0.22 s before each shot.
  timeout: 120_000,
  expect: { timeout: 10_000 },
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["github"]] : "list",
  use: { baseURL: `http://localhost:${PORT}`, trace: "retain-on-failure" },
  // Never another server that happens to be on the port: the tests would run against its build.
  webServer: { command: "node e2e/serve.mjs", port: PORT, reuseExistingServer: false, env: { PORT: String(PORT) } },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
    { name: "firefox", use: { ...devices["Desktop Firefox"] } },
    { name: "phone", use: { ...devices["iPhone 15"] } },
  ],
});
