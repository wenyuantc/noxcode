import fs from "fs";
import path from "path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const PDFJS_DIRS = ["cmaps", "standard_fonts", "wasm"] as const;

function pdfjsAssets(): Plugin {
  const pkgRoot = path.resolve("node_modules/pdfjs-dist");
  return {
    name: "pdfjs-assets",
    configureServer(server) {
      server.middlewares.use("/pdfjs", (req, res, next) => {
        const parts = decodeURIComponent((req.url ?? "").split("?")[0])
          .split("/")
          .filter(Boolean);
        const dir = parts[0];
        const name = parts[1];
        if (
          parts.length !== 2 ||
          !PDFJS_DIRS.includes(dir as (typeof PDFJS_DIRS)[number]) ||
          !name ||
          name.includes("..")
        ) {
          next();
          return;
        }
        const file = path.join(pkgRoot, dir, name);
        if (!fs.existsSync(file) || !fs.statSync(file).isFile()) {
          next();
          return;
        }
        const type =
          path.extname(name) === ".wasm"
            ? "application/wasm"
            : path.extname(name) === ".ttf"
              ? "font/ttf"
              : "application/octet-stream";
        res.setHeader("Content-Type", type);
        fs.createReadStream(file).pipe(res);
      });
    },
    generateBundle() {
      for (const dir of PDFJS_DIRS) {
        const from = path.join(pkgRoot, dir);
        for (const name of fs.readdirSync(from)) {
          const full = path.join(from, name);
          if (!fs.statSync(full).isFile()) continue;
          this.emitFile({
            type: "asset",
            fileName: `pdfjs/${dir}/${name}`,
            source: fs.readFileSync(full),
          });
        }
      }
    },
  };
}

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [react(), tailwindcss(), pdfjsAssets()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
