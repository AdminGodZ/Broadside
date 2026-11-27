import { pathToFileURL } from "node:url";
import { join } from "node:path";
import { expect, playOut, test } from "./fixtures";

// dist/broadside.html: the game in one file (scripts/offline.mjs), opened from disk.
const FILE = pathToFileURL(join(import.meta.dirname, "..", "dist", "broadside.html")).href;

test("the offline file plays from disk", async ({ page }) => {
  await page.goto(FILE);
  await expect(page.locator("#home .layer .hull")).toHaveCount(5);
  await page.locator("#start").click();
  await expect(page.locator("#target-col")).toBeVisible();
  expect((await playOut(page, 8)).engineShots).toBe(8);
  // Its links go back to the site.
  await expect(page.locator('a[href^="/"]')).toHaveCount(0);
});
