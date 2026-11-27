import { expect, test } from "./fixtures";

test("the home page shows, and leads to the game", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle("Broadside");
  await expect(page.locator("h1")).toBeVisible();
  // The page is never left hidden waiting for its font (Base.astro).
  await expect(page.locator("html")).not.toHaveClass(/fonts-wait/);
  await page.locator('main a[href="/play"]').first().click();
  await expect(page).toHaveURL(/\/play$/);
  await expect(page.locator("#home .layer .hull")).toHaveCount(5);
});

test("the download section offers a build for each system", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("#download-main")).toHaveAttribute("href", /github\.com\/.+\/releases/);
  await page.locator("details.systems summary").click();
  await expect(page.locator("details.systems li a")).not.toHaveCount(0);
});

for (const path of ["/", "/play"]) {
  test(`${path} fits a phone's width`, async ({ page }) => {
    await page.setViewportSize({ width: 360, height: 740 });
    await page.goto(path);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow).toBeLessThanOrEqual(0);
  });
}
