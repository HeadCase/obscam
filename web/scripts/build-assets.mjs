import { copyFile, mkdir } from "node:fs/promises";

await mkdir(new URL("../dist/", import.meta.url), { recursive: true });
await Promise.all([
  copyFile(
    new URL("../static/index.html", import.meta.url),
    new URL("../dist/index.html", import.meta.url)
  ),
  copyFile(
    new URL("../static/styles.css", import.meta.url),
    new URL("../dist/styles.css", import.meta.url)
  )
]);
