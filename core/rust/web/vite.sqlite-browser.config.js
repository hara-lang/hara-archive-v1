import { defineConfig } from "vite";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const web = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  resolve: {
    alias: {
      "@hara-lang/db-sqlite": resolve(web, "packages/db-sqlite/index.mjs")
    }
  },
  build: {
    target: "es2022",
    outDir: "dist-sqlite-browser",
    emptyOutDir: true,
    assetsInlineLimit: 0,
    lib: {
      entry: resolve(web, "entries/sqlite-browser.mjs"),
      formats: ["es"],
      fileName: () => "provider.mjs"
    },
    rollupOptions: {
      output: {
        assetFileNames: "assets/[name][extname]"
      }
    }
  }
});
