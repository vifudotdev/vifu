import { mkdir, rm, copyFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const srcDir = path.join(root, "src");
const distDir = path.join(root, "dist");
const browserDir = path.join(distDir, "browser");

await rm(distDir, { recursive: true, force: true });
await mkdir(browserDir, { recursive: true });
await copyFile(path.join(srcDir, "index.d.ts"), path.join(distDir, "index.d.ts"));

await build({
  entryPoints: [path.join(srcDir, "index.js")],
  outfile: path.join(distDir, "index.js"),
  bundle: true,
  format: "esm",
  platform: "browser",
  target: "es2020",
  sourcemap: false,
  logLevel: "silent",
});

await build({
  entryPoints: [path.join(srcDir, "browser-entry.js")],
  outfile: path.join(browserDir, "vifu-sdk.js"),
  bundle: true,
  format: "iife",
  platform: "browser",
  target: "es2020",
  sourcemap: false,
  logLevel: "silent",
});
