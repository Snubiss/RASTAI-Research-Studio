// The web app's build (layout frontend-and-plugins.md 8.1). `vite build` writes dist/, which rastai-server embeds
// (build.rs, `just web-embed`). Nothing is loaded from a CDN: fonts and styles are bundled, and the output is hashed,
// so a new build is a new file and an old index.html can't load a mismatched one.

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "vite";

const here = dirname(fileURLToPath(import.meta.url));
// The daemon's own address: the dev proxy talks to it, and it refuses any Origin but its own.
const DAEMON_ADDRESS = "127.0.0.1:47820";
const DAEMON_ORIGIN = `http://${DAEMON_ADDRESS}`;
// Hashed assets are small enough to inline otherwise; a data: URI would need the CSP to allow it for fonts, so
// nothing is inlined.
const NO_INLINED_ASSETS = 0;

/**
 * The workspace version, which the daemon also reports on /api/health. The app compares the two (RRS-U0004).
 * @returns {string}
 */
function workspaceVersion() {
  const cargoToml = readFileSync(resolve(here, "../../Cargo.toml"), "utf8");
  const match = cargoToml.match(/^version = "([^"]+)"/m);
  if (match === null) {
    throw new Error("Cargo.toml has no workspace version");
  }
  return match[1];
}

export default defineConfig({
  // The JSX of every .js and .jsx file is the automatic runtime: no React import in each component.
  esbuild: { jsx: "automatic" },
  // Read by src/constants.js as import.meta.env.RASTAI_APP_VERSION, and compared with the daemon's (RRS-U0004).
  define: { "import.meta.env.RASTAI_APP_VERSION": JSON.stringify(workspaceVersion()) },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    assetsInlineLimit: NO_INLINED_ASSETS,
    modulePreload: { polyfill: false },
    sourcemap: false,
  },
  server: {
    // `vite` serves the app on its own port; the daemon refuses requests whose Origin isn't its own, so the proxy
    // rewrites Host and Origin on every request (layout 8.4, dev loop). Sessions then match the daemon's origin.
    proxy: {
      "/api": { target: DAEMON_ORIGIN, changeOrigin: true, headers: { Origin: DAEMON_ORIGIN } },
      "/rastai.": { target: DAEMON_ORIGIN, changeOrigin: true, headers: { Origin: DAEMON_ORIGIN } },
    },
  },
});
