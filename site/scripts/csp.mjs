// After `astro build` and offline.mjs: gives every page in dist/ its
// Content-Security-Policy, as a <meta> first in the head. The pages are static,
// so the policy names exactly what each one holds: every inline script and
// stylesheet by its hash, and beyond those only the site's own files. A script
// or style that gets into a page some other way does not run.
//
// Computed here from the built pages rather than by Astro (security.csp): Astro
// hashes the scripts it bundles, not the `is:inline` ones the first paint relies
// on, and the offline file is put together after it has finished.
//
// vercel.json sends the rest as a header (what a <meta> cannot say, such as who
// may frame the site).
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const DIST = join(import.meta.dirname, "..", "dist");
const BLOCKS = /<(script|style)\b([^>]*)>([\s\S]*?)<\/\1\s*>/gi;
const META = /<meta http-equiv="content-security-policy"[^>]*>/i;
const CHARSET = /<meta charset="utf-8"\s*\/?>/i;

const fail = (why) => {
  console.error(`csp.mjs: ${why}`);
  process.exit(1);
};
// What a browser hashes: the text between the tags, as it stands in the file.
const hash = (text) => `'sha256-${createHash("sha256").update(text).digest("base64")}'`;

function policy(file, html) {
  const scripts = new Set(), styles = new Set();
  for (const [, tag, attrs, body] of html.matchAll(BLOCKS)) {
    if (tag.toLowerCase() === "style") styles.add(hash(body));
    else if (!/\ssrc=/i.test(attrs)) scripts.add(hash(body));
  }
  // Hashes cover <script> and <style>, not attributes: a style="" or an onclick="" in the
  // markup would be turned away by the browser, quietly. Stop the build instead.
  const attribute = html.replace(BLOCKS, "").match(/<[a-z][^>]*\s(?:style|on[a-z]+)=[^>]*>/i);
  if (attribute) fail(`${file} has an inline style or handler, which the policy turns away (use a class, or a listener): ${attribute[0].slice(0, 200)}`);

  const linked = /<link\b[^>]*rel="stylesheet"/i.test(html);
  return [
    "default-src 'none'",
    // The engine is WebAssembly, compiled on the page.
    `script-src 'self' 'wasm-unsafe-eval' ${[...scripts].join(" ")}`.trimEnd(),
    `style-src ${[linked ? "'self'" : "", ...styles].filter(Boolean).join(" ") || "'none'"}`,
    // data: for the paper grain and the stencil face (global.css), and all of both in the offline file.
    "img-src 'self' data:",
    "font-src 'self' data:",
    "connect-src 'self'", // the engine, and /play fetched ahead (Base.astro)
    "worker-src blob:", // the engine's thread (engine.ts)
    "base-uri 'none'",
    "form-action 'none'",
  ].join("; ");
}

const pages = readdirSync(DIST, { recursive: true }).filter((f) => f.endsWith(".html"));
if (!pages.length) fail("no pages in dist/");
for (const file of pages) {
  const path = join(DIST, file);
  const html = readFileSync(path, "utf8").replace(META, "");
  if (!CHARSET.test(html)) fail(`${file} has no <meta charset> to put the policy after`);
  // First in the head, after the charset: a policy holds only for what comes after it.
  writeFileSync(path, html.replace(CHARSET, (charset) => `${charset}<meta http-equiv="content-security-policy" content="${policy(file, html)}">`));
}
console.log(`csp.mjs: ${pages.length} pages (${pages.join(", ")})`);
