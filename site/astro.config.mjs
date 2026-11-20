import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://broadside.admingod.ch",
  output: "static",
  devToolbar: { enabled: false },
  vite: {
    // The logo is imported from the repository's assets/ folder, outside this project.
    server: { fs: { allow: [".."] } },
  },
});
