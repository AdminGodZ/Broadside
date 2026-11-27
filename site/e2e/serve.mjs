// Serves dist/ for the end-to-end tests as Vercel serves it: clean addresses
// (/play is dist/play/index.html), no trailing slash, and the headers vercel.json
// sets (the security headers; assets under /_astro/ cached for good, while pages
// are checked every time).
import { createReadStream, existsSync, readFileSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize } from "node:path";

const DIST = join(import.meta.dirname, "..", "dist");
const PORT = Number(process.env.PORT ?? 4329);
// vercel.json's header rules; their sources ("/(.*)", "/_astro/(.*)") read as regular expressions too.
const RULES = JSON.parse(readFileSync(join(import.meta.dirname, "..", "..", "vercel.json"), "utf8")).headers.map(
  ({ source, headers }) => ({ source: new RegExp(`^${source}$`), headers: headers.map(({ key, value }) => [key.toLowerCase(), value]) }),
);
const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript",
  ".css": "text/css",
  ".wasm": "application/wasm",
  ".woff2": "font/woff2",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".txt": "text/plain",
  ".xml": "application/xml",
};

createServer((req, res) => {
  const path = decodeURIComponent(new URL(req.url, "http://x").pathname);
  if (path.length > 1 && path.endsWith("/")) return res.writeHead(308, { location: path.slice(0, -1) }).end();
  const base = normalize(join(DIST, path));
  if (!base.startsWith(DIST)) return res.writeHead(403).end();
  const file = [base, `${base}.html`, join(base, "index.html")].find((f) => existsSync(f) && statSync(f).isFile());
  if (!file) return res.writeHead(404).end();
  const headers = { "content-type": TYPES[extname(file)] ?? "application/octet-stream", "cache-control": "public, max-age=0, must-revalidate" };
  for (const rule of RULES) if (rule.source.test(path)) Object.assign(headers, Object.fromEntries(rule.headers));
  res.writeHead(200, headers);
  createReadStream(file).pipe(res);
}).listen(PORT, () => console.log(`e2e: serving dist on http://localhost:${PORT}`));
