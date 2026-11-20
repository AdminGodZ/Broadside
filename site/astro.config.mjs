import { defineConfig, fontProviders } from "astro/config";

// Fonts come from the installed Fontsource packages (no network at build time).
// Base.astro preloads them, so they arrive with the page instead of after the
// stylesheet, and the fallbacks are metric-matched so text doesn't move.
// Latin only: the site is in English, and every file declared here is preloaded.
const LATIN =
  "U+0000-00FF,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+0304,U+0308,U+0329,U+2000-206F,U+20AC,U+2122,U+2191,U+2193,U+2212,U+2215,U+FEFF,U+FFFD";
const NEWSREADER = "@fontsource-variable/newsreader/files/newsreader";
const SAIRA = "@fontsource/saira-stencil-one/files/saira-stencil-one";

export default defineConfig({
  site: "https://broadside.admingod.ch",
  output: "static",
  devToolbar: { enabled: false },
  fonts: [
    {
      name: "Newsreader",
      cssVariable: "--serif",
      provider: fontProviders.local(),
      fallbacks: ["Iowan Old Style", "Palatino Linotype", "Georgia", "serif"],
      // A preloaded file normally arrives within the 100 ms block; on a slow
      // connection the fallback shows rather than blank text.
      display: "fallback",
      options: {
        variants: [
          { src: [`${NEWSREADER}-latin-opsz-normal.woff2`], weight: "200 800", style: "normal", unicodeRange: [LATIN] },
          { src: [`${NEWSREADER}-latin-opsz-italic.woff2`], weight: "200 800", style: "italic", unicodeRange: [LATIN] },
        ],
      },
    },
    {
      name: "Saira Stencil One",
      cssVariable: "--stencil",
      provider: fontProviders.local(),
      fallbacks: ["Arial Narrow", "Impact", "sans-serif"],
      // The name only: better a moment blank than in another typeface.
      display: "block",
      options: {
        variants: [
          { src: [`${SAIRA}-latin-400-normal.woff2`], weight: "400", style: "normal", unicodeRange: [LATIN] },
        ],
      },
    },
  ],
  vite: {
    // The logo is imported from the repository's assets/ folder, outside this project.
    server: { fs: { allow: [".."] } },
  },
});
