import { expect, playOut, saveFleet, test } from "./fixtures";

const OVER = /^(Victory in \d+ shots|The engine wins in \d+ shots)$/;

test.beforeEach(async ({ page }) => saveFleet(page));

test("a whole game: started before the engine is in, played out, then another", async ({ page }) => {
  await page.goto("/play", { waitUntil: "commit" });
  // Start battle as soon as it shows: it waits for the engine, which only loads after the page.
  await page.locator("#start").click();
  await expect(page.locator("#target-col")).toBeVisible();
  await expect(page.locator("#log li").last()).toHaveText(/^Coin toss: /);

  const game = await playOut(page);
  expect(game.over).toBe(true);
  expect(game.heading).toMatch(OVER);
  // The engine's fleet is shown once it's over, sunk or outlined.
  await expect(page.locator("#target .layer .hull")).toHaveCount(5);
  const record = Number(await page.locator("#rec-w").textContent()) + Number(await page.locator("#rec-l").textContent());
  expect(record).toBe(1);
  await expect(page.locator("#resign")).toBeHidden();

  await page.locator("#new-game").click();
  await expect(page.locator("#dock")).toBeVisible();
  await expect(page.locator("#banner")).toBeHidden();
  await expect(page.locator("#log li")).toHaveCount(0);
  await expect(page.locator("#msg .msg-sub")).toHaveText(/^Your last fleet is in position\./);
  await page.locator("#start").click();
  await expect(page.locator("#target-col")).toBeVisible();
  expect((await playOut(page, 3)).engineShots).toBe(3);
});

test("Start battle pressed before the game's script is in still starts it", async ({ page }) => {
  // The script held back a second (a slow connection): the page has drawn the fleet and enabled
  // Start on its own (Game.astro), so a press then must not be lost.
  let released = false;
  await page.route(/\/_astro\/Game\.astro.*\.js$/, async (route) => {
    await new Promise((done) => setTimeout(done, 1000));
    released = true;
    await route.continue();
  });
  await page.goto("/play", { waitUntil: "commit" });
  await expect(page.locator("#start")).toBeEnabled();
  await page.locator("#start").click();
  expect(released, "pressed before the script came").toBe(false);
  await expect(page.locator("#target-col")).toBeVisible();
  await expect(page.locator("#log li").last()).toHaveText(/^Coin toss: /);
});

test("the heat map shows where the engine expects your ships, and follows its shots", async ({ page }) => {
  await page.goto("/play");
  await page.locator("#start").click();
  await page.locator("#heat").check();
  await expect(page.locator("#heat-scale")).toBeVisible();
  await expect(page.locator("#home .cell .heat").first()).toBeAttached();
  await playOut(page, 5);
  // Every cell the engine hasn't fired at carries its chance; the five it has, none.
  await expect(page.locator("#home .cell .heat")).toHaveCount(95);
  await expect(page.locator("#home .cell .pct").first()).toHaveText(/^[A-J]\d{1,2}: (\d+%|under 1%|over 99%)$/);
  await page.locator("#heat").uncheck();
  await expect(page.locator("#home .cell .heat")).toHaveCount(0);
});

test("resigning while the engine aims: its shot never lands", async ({ page }) => {
  await page.goto("/play");
  await page.locator("#start").click();
  await expect(page.locator("#target-col")).toBeVisible();
  // In the page, so the resign comes inside the engine's 0.22 s.
  const aiming = await page.evaluate(async () => {
    const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms));
    let open: HTMLButtonElement | null;
    while (!(open = document.querySelector("#target .cell.open"))) await sleep(5);
    open.click();
    while (!document.getElementById("msg")!.textContent!.includes("aiming")) await sleep(1);
    document.getElementById("resign")!.click();
    return true;
  });
  expect(aiming).toBe(true);
  await expect(page.locator("#banner h3")).toHaveText(/^The engine wins in \d+ shots$/);
  await page.locator("#new-game").click();
  await page.waitForTimeout(600); // past when the engine's shot would have landed
  await expect(page.locator("#log li")).toHaveCount(0);
  await expect(page.locator("#home .cell.hit, #home .cell.miss")).toHaveCount(0);
  await expect(page.locator("#dock")).toBeVisible();
});

test("without a worker, the engine runs on the page", async ({ page }) => {
  await page.addInitScript(() => {
    (window as { Worker?: unknown }).Worker = undefined;
  });
  await page.goto("/play");
  await page.locator("#start").click();
  const game = await playOut(page);
  expect(game.heading).toMatch(OVER);
  await expect(page.locator("#target .layer .hull")).toHaveCount(5);
});

test("winning: the banner and the record say so", async ({ page }) => {
  // The test can't outplay the engine, so on the page's thread (no worker) the engine is told the
  // player has won after three shots.
  await page.addInitScript(() => {
    (window as { Worker?: unknown }).Worker = undefined;
    const instantiate = WebAssembly.instantiate.bind(WebAssembly) as (...a: unknown[]) => Promise<WebAssembly.Instance>;
    let shots = 0;
    (WebAssembly as { instantiate: unknown }).instantiate = async (...args: unknown[]) => {
      const instance = await instantiate(...args);
      const real = instance.exports as Record<string, (...a: number[]) => number>;
      const exports = { ...real, bs_player_fire: (c: number) => (shots++, real.bs_player_fire(c)), bs_player_won: () => (shots >= 3 ? 1 : 0) };
      return { exports } as unknown as WebAssembly.Instance;
    };
  });
  await page.goto("/play");
  await page.locator("#start").click();
  const game = await playOut(page);
  expect(game.heading).toBe("Victory in 3 shots");
  await expect(page.locator("#rec-w")).toHaveText("1");
  await expect(page.locator("#rec-avg")).toHaveText("3.0");
});

test("an engine that can't load says so", async ({ page, errors }) => {
  await page.route(/\.wasm$/, (route) => route.abort());
  await page.goto("/play");
  await expect(page.locator(".error")).toHaveText(/^The engine could not start: /);
  // The failed request is the only error, and it's expected here.
  expect(errors.every((e) => /fail|wasm|network|load/i.test(e))).toBe(true);
  errors.length = 0;
});
