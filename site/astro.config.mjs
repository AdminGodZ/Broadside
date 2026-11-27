import tailwindcss from "@tailwindcss/vite";
import { defineConfig, fontProviders } from "astro/config";

import { FILES, UNICODE_RANGE, subsetFonts } from "./scripts/fonts.mjs";

// Fonts: Newsreader from the installed Fontsource package (no network at build
// time), cut down by scripts/fonts.mjs to the characters, weights and styles the
// site uses. Base.astro preloads it, so it arrives with the page; the stencil face
// for the name is small enough to go inside the page (src/fonts/stencil.css).
await subsetFonts();

export default defineConfig({
  site: "https://broadside.admingod.ch",
  output: "static",
  devToolbar: { enabled: false },
  // The stylesheet goes into each page: no request holds up the first paint.
  build: { inlineStylesheets: "always" },
  fonts: [
    {
      name: "Newsreader",
      cssVariable: "--serif",
      provider: fontProviders.local(),
      // Metric-matched by Astro, so the fallback takes the same room.
      fallbacks: ["Iowan Old Style", "Palatino Linotype", "Georgia", "serif"],
      // The page waits for the font before it shows (Base.astro), so this is for the italic,
      // which isn't waited for: its caption appears when it's in rather than moving then.
      display: "block",
      options: {
        variants: [
          { src: [FILES.normal], weight: "400 600", style: "normal", unicodeRange: [UNICODE_RANGE] },
          { src: [FILES.italic], weight: "400", style: "italic", unicodeRange: [UNICODE_RANGE] },
        ],
      },
    },
  ],
  vite: {
    plugins: [tailwindcss()],
    // The logo is imported from the repository's assets/ folder, outside this project.
    server: { fs: { allow: [".."] } },
  },
});
