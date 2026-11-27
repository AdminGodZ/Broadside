import { test as base, expect, type Page } from "@playwright/test";

/** Every test fails if the page throws or logs an error, unless it takes them out of `errors` itself. */
export const test = base.extend<{ errors: string[] }>({
  errors: [
    async ({ page }, use) => {
      const errors: string[] = [];
      page.on("pageerror", (e) => errors.push(`uncaught: ${e.message}`));
      page.on("console", (m) => {
        // WebKit turns down the site's prefetch rules (Base.astro) over plain http, as the tests serve it.
        if (m.type() === "error" && !m.text().startsWith("Prefetch request denied: URL must be secure")) errors.push(`console: ${m.text()}`);
      });
      await use(errors);
      expect(errors, "errors on the page").toEqual([]);
    },
    { auto: true },
  ],
});
export { expect };

/** A fleet by placement slot (top-left cell * 2 + down): Carrier A1–A5, Battleship B1–B4, Cruiser C1–C3,
 * Submarine D1–D3, Destroyer E1–E2, all across. */
export const FLEET = [0, 20, 40, 60, 80];
export const FLEET_SPANS = ["A1–A5", "B1–B4", "C1–C3", "D1–D3", "E1–E2"];

/** Starts every page in this test with `fleet` saved, as if the player had gone into battle with it before. */
export const saveFleet = (page: Page, fleet = FLEET) =>
  page.addInitScript((f) => localStorage.setItem("broadside.fleet", JSON.stringify(f)), fleet);

export const homeCell = (page: Page, i: number) => page.locator("#home .cell").nth(i);
export const homeHulls = (page: Page) => page.locator("#home .layer .hull");

/** Where each hull on your chart is drawn, by its inline style. */
export const hullStyles = (page: Page) =>
  homeHulls(page).evaluateAll((hulls) => hulls.map((h) => (h as HTMLElement).style.cssText));

/**
 * Plays the battle out from the page's side: fires at a random open cell whenever it's the
 * player's turn, until the banner says who won. Runs in the page, so a game takes as long as
 * the engine does, not one round trip to the test per shot. `stopAfter` ends it early, after
 * that many engine shots.
 */
export async function playOut(page: Page, stopAfter = Infinity) {
  return page.evaluate(async (stopAfter) => {
    const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms));
    const banner = document.getElementById("banner")!;
    const engineShots = () => document.querySelectorAll("#log li.it").length;
    const deadline = Date.now() + 100_000;
    while (banner.hidden && engineShots() < stopAfter) {
      if (Date.now() > deadline) throw new Error("the game did not finish");
      const open = [...document.querySelectorAll<HTMLButtonElement>("#target .cell.open")];
      if (open.length) open[Math.floor(Math.random() * open.length)].click();
      await sleep(open.length ? 10 : 5);
    }
    return { over: !banner.hidden, heading: banner.querySelector("h3")?.textContent ?? "", engineShots: engineShots() };
  }, stopAfter);
}
