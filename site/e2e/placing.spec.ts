import { FLEET_SPANS, expect, homeCell, homeHulls, hullStyles, saveFleet, test } from "./fixtures";

const SPAN = /^[A-J]\d{1,2}–[A-J]\d{1,2}$/;

test.describe("the first frame", () => {
  test("the page draws the fleet itself, without the script", async ({ page, errors }) => {
    // The game's script blocked: what's on screen is the markup and its inline script (Game.astro).
    await page.route(/\/_astro\/Game\.astro.*\.js$/, (route) => route.abort());
    await page.goto("/play");
    await expect(homeHulls(page)).toHaveCount(5);
    await expect(page.locator("#dock-ships .ship-row.placed")).toHaveCount(5);
    for (const where of await page.locator("#dock-ships .where").allTextContents()) expect(where).toMatch(SPAN);
    await expect(page.locator("#it-shots")).toHaveText("5 of 5 placed");
    await expect(page.locator("#start")).toBeEnabled();
    await expect(page.locator("#msg")).toContainText("Your fleet is ready");
    await expect(page.locator("#msg .msg-sub")).toHaveText(/^A random fleet\./);
    // The blocked script is the only error, and it's expected here.
    expect(errors.every((e) => /fail|load|network/i.test(e))).toBe(true);
    errors.length = 0;
  });

  test("the script takes over the fleet the page drew, as it is", async ({ page }) => {
    // The hulls as the page's own script first draws them, before game.ts runs.
    await page.addInitScript(() => {
      new MutationObserver((_, observer) => {
        const hulls = document.querySelectorAll<HTMLElement>("#home .layer .hull");
        if (hulls.length < 5) return;
        (window as unknown as { firstHulls: string[] }).firstHulls = [...hulls].map((h) => h.style.cssText);
        observer.disconnect();
      }).observe(document, { childList: true, subtree: true });
    });
    await page.goto("/play");
    await expect(page.locator("#dock-ships .ship-row.placed")).toHaveCount(5);
    await page.waitForLoadState("load");
    const first = await page.evaluate(() => (window as unknown as { firstHulls: string[] }).firstHulls);
    expect(first).toHaveLength(5);
    expect(await hullStyles(page)).toEqual(first);
    // Taken over, not dropped in again.
    await expect(page.locator("#home .hull.fresh")).toHaveCount(0);
  });

  test("a saved fleet comes back where it was", async ({ page }) => {
    await saveFleet(page);
    await page.goto("/play");
    await expect(page.locator("#dock-ships .where")).toHaveText(FLEET_SPANS);
    await expect(page.locator("#msg .msg-sub")).toHaveText(/^Your last fleet is in position\./);
    await expect(homeHulls(page).first()).toHaveAttribute("style", /left: 0%; top: 0%; width: 50%; height: 10%/);
  });
});

test.describe("placing the fleet", () => {
  test.beforeEach(async ({ page }) => {
    await saveFleet(page);
    await page.goto("/play");
    await page.waitForLoadState("load");
  });

  test("ships can be picked up, turned, moved and put back", async ({ page }) => {
    const rows = page.locator("#dock-ships .ship-row");
    // Pick up the Carrier from A1 and turn it down.
    await homeCell(page, 0).click();
    await expect(rows.nth(0)).toHaveClass(/\bsel\b/);
    await expect(page.locator("#start")).toBeDisabled();
    await page.locator("#dir-v").click();
    await expect(page.locator("#dir-v")).toHaveAttribute("aria-pressed", "true");
    // Down from F10.
    await homeCell(page, 59).click();
    await expect(rows.nth(0).locator(".where")).toHaveText("F10–J10");
    await expect(page.locator("#start")).toBeEnabled();

    // The Battleship, down from D10, would cross the Carrier at F10.
    await rows.nth(1).click();
    await homeCell(page, 39).click();
    await expect(page.locator("#msg")).toContainText("It would overlap another ship.");
    // Down from H9 it runs off the chart.
    await homeCell(page, 78).click();
    await expect(page.locator("#msg")).toContainText("runs off the chart");
    // Picking it again puts it back where it was.
    await rows.nth(1).click();
    await expect(rows.nth(1).locator(".where")).toHaveText(FLEET_SPANS[1]);
    await expect(rows.nth(1)).not.toHaveClass(/\bsel\b/);
  });

  test("Escape and the R key work while placing", async ({ page, isMobile }) => {
    test.skip(isMobile, "keyboard");
    await homeCell(page, 10).click(); // the Battleship, B1–B4
    await page.keyboard.press("r");
    await expect(page.locator("#dir-v")).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.press("Escape");
    await expect(page.locator("#dock-ships .ship-row.sel")).toHaveCount(0);
    await expect(page.locator("#dock-ships .where").nth(1)).toHaveText(FLEET_SPANS[1]);
  });

  test("Clear empties the chart, Random fills it", async ({ page }) => {
    await page.locator("#clear").click();
    await expect(homeHulls(page)).toHaveCount(0);
    await expect(page.locator("#it-shots")).toHaveText("0 of 5 placed");
    await expect(page.locator("#start")).toBeDisabled();
    await page.locator("#random").click();
    await expect(homeHulls(page)).toHaveCount(5);
    await expect(page.locator("#start")).toBeEnabled();
    await expect(page.locator("#dock-ships .ship-row.placed")).toHaveCount(5);
  });
});
