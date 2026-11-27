import { readFileSync } from "node:fs";
import { join } from "node:path";

// The engine's version, read from the workspace Cargo.toml at build time, so the
// site always names the version it was built from.
const cargo = readFileSync(join(process.cwd(), "..", "Cargo.toml"), "utf8");
const version = /^version\s*=\s*"([^"]+)"/m.exec(cargo)?.[1];
if (!version) throw new Error("no version in ../Cargo.toml");
export const VERSION = version;

export const REPO = "https://github.com/AdminGodZ/Broadside";
export const RELEASES = `${REPO}/releases`;
const LATEST = `${REPO}/releases/latest/download`;

/** Built by .github/workflows/release.yml; file names carry no version. */
export const SYSTEMS = [
  { id: "mac-arm", name: "macOS, Apple Silicon", button: "Download for Mac", os: "mac", url: `${LATEST}/broadside-aarch64-apple-darwin.tar.gz` },
  { id: "mac-intel", name: "macOS, Intel", button: "Download for Mac", os: "mac", url: `${LATEST}/broadside-x86_64-apple-darwin.tar.gz` },
  { id: "windows", name: "Windows", button: "Download for Windows", os: "windows", url: `${LATEST}/broadside-x86_64-pc-windows-msvc.zip` },
  { id: "linux", name: "Linux, x86-64", button: "Download for Linux", os: "linux", url: `${LATEST}/broadside-x86_64-unknown-linux-gnu.tar.gz` },
  { id: "linux-arm", name: "Linux, ARM", button: "Download for Linux", os: "linux", url: `${LATEST}/broadside-aarch64-unknown-linux-gnu.tar.gz` },
] as const;

/** The play page as one self-contained file (scripts/offline.mjs), served by the site itself:
 * dist/broadside.html, at /broadside without the extension (vercel.json's cleanUrls; the
 * .html address redirects there). Links set download="broadside.html" for the saved name. */
export const OFFLINE_GAME = "/broadside";
