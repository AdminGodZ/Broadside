// After `astro build`: folds the play page into one self-contained file,
// dist/broadside.html, that runs offline from disk. Stylesheets, fonts, images,
// the script and the engine (WebAssembly, as base64) all go inline; links back
// to the site become absolute. The site serves it for "Save it for offline play",
// and the release attaches it.
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const DIST = join(import.meta.dirname, "..", "dist");
const SITE = "https://broadside.admingod.ch";
const TYPES = { woff2: "font/woff2", svg: "image/svg+xml", png: "image/png", wasm: "application/wasm" };

const file = (url) => readFileSync(join(DIST, url));
const dataUri = (url) => `data:${TYPES[url.split(".").pop()] ?? "application/octet-stream"};base64,${file(url).toString("base64")}`;
// Every /_astro/ asset a stylesheet or the markup points at, as a data URI.
const inlineAssets = (text) => text.replace(/\/_astro\/[\w./-]+\.(?:woff2|svg|png)/g, dataUri);
const fail = (why) => {
  console.error(`offline.mjs: ${why}`);
  process.exit(1);
};

let html = readFileSync(join(DIST, "play", "index.html"), "utf8");

// The script: one module, with no imports of its own (a split chunk would need bundling here).
const scripts = [...html.matchAll(/<script type="module" src="(\/_astro\/[^"]+\.js)"><\/script>/g)];
if (scripts.length !== 1) fail(`expected one module script on /play, found ${scripts.length}`);
const js = file(scripts[0][1]).toString("utf8");
if (/(^|[;\s])import[\s{(*"']/.test(js.replace(/import\.meta/g, ""))) fail("the play script imports another chunk");
const wasm = js.match(/\/_astro\/[\w.-]+\.wasm/)?.[0] ?? fail("no engine (.wasm) referenced by the play script");
html = html.replace(
  scripts[0][0],
  `<script>window.BROADSIDE_WASM = "${file(wasm).toString("base64")}";</script>\n<script type="module">${js.replace(/<\/script/gi, "<\\/script")}</script>`,
);

// Stylesheets inline; font and engine preloads dropped (both are inline now).
html = html.replace(/<link rel="stylesheet" href="(\/_astro\/[^"]+\.css)"\s*\/?>/g, (_, url) => `<style>${inlineAssets(file(url).toString("utf8"))}</style>`);
html = html.replace(/<link rel="preload"[^>]*as="(?:font|fetch)"[^>]*>/g, "");
// Fetching the site's pages ahead (Base.astro) means nothing in a file opened from disk.
html = html.replace(/<script\b[^>]*>(?:(?!<\/script>)[\s\S])*<\/script>/g, (tag) => (tag.includes("speculationrules") ? "" : tag));
html = inlineAssets(html);

// Links back to the site, absolute; a canonical link is meaningless in a local file.
html = html.replace(/<link rel="canonical"[^>]*>/, "");
html = html.replace(/(href)="\/([^"/][^"]*)?"/g, (_, attr, path = "") => `${attr}="${SITE}/${path}"`);

if (/(?:src|href)="\/_astro\//.test(html)) fail("an /_astro/ reference was left behind");
writeFileSync(join(DIST, "broadside.html"), html);
console.log(`offline.mjs: dist/broadside.html, ${(html.length / 1e6).toFixed(2)} MB`);
