import { copyFile, mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

await mkdir(new URL("../dist/", import.meta.url), { recursive: true });
await Promise.all([
  copyFile(
    new URL("../static/index.html", import.meta.url),
    new URL("../dist/index.html", import.meta.url)
  ),
  copyFile(
    new URL("../static/styles.css", import.meta.url),
    new URL("../dist/styles.css", import.meta.url)
  ),
  build({
    entryPoints: [fileURLToPath(new URL("../src/app.ts", import.meta.url))],
    outfile: fileURLToPath(new URL("../dist/app.js", import.meta.url)),
    bundle: true,
    format: "esm",
    platform: "browser",
    target: "es2022",
    minify: true,
    logLevel: "warning"
  })
]);
