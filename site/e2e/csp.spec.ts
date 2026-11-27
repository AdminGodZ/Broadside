import { expect, test } from "./fixtures";

// The Content-Security-Policy: each page's own, written into it by scripts/csp.mjs, and the
// header vercel.json adds. That it lets the site's own scripts, styles and engine through is
// what every other test shows: a browser logs what it turns away, and a logged error fails a test.
for (const path of ["/", "/play", "/broadside"]) {
  test(`${path} turns away a script that isn't the site's`, async ({ page, errors }) => {
    const response = await page.goto(path);
    expect(response!.headers()["content-security-policy"]).toContain("frame-ancestors 'none'");
    await expect(page.locator('meta[http-equiv="content-security-policy"]')).toHaveAttribute("content", /^default-src 'none'; script-src 'self' 'wasm-unsafe-eval' 'sha256-/);

    const refused = await page.evaluate(
      () =>
        new Promise<string>((done) => {
          document.addEventListener("securitypolicyviolation", (e) => done(e.violatedDirective), { once: true });
          const script = document.createElement("script");
          script.textContent = "window.injected = true";
          document.head.append(script);
          setTimeout(() => done("nothing was refused"), 2000);
        }),
    );
    expect(refused).toMatch(/^script-src/);
    expect(await page.evaluate(() => "injected" in window)).toBe(false);
    errors.length = 0; // the browser logs the refusal
  });
}
