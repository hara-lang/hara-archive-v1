import { defineConfig } from "vite";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const web = dirname(fileURLToPath(import.meta.url));
const provider = resolve(web, "../../../providers/webdav/hta");

export default defineConfig({
  resolve: {
    alias: {
      "@hara-lang/fs-webdav": resolve(provider, "index.mjs")
    }
  },
  build: {
    target: "es2022",
    outDir: resolve(provider, "browser"),
    emptyOutDir: true,
    assetsInlineLimit: 0,
    lib: {
      entry: resolve(web, "entries/webdav-browser.mjs"),
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
