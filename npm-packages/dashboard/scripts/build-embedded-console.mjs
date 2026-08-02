import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const workspaceRoot = path.resolve(packageRoot, "../..");
const outputRoot = path.resolve(
  workspaceRoot,
  process.env.VIFU_CONSOLE_ASSETS_DIR ?? "target/vifu-console-assets",
);
const assetsDir = path.join(outputRoot, "assets");

rmSync(outputRoot, { recursive: true, force: true });
mkdirSync(assetsDir, { recursive: true });

run("bun", [
  "build",
  "embedded/main.tsx",
  "--target=browser",
  "--format=esm",
  "--outdir",
  assetsDir,
  "--entry-naming",
  "[name]-[hash].[ext]",
  "--asset-naming",
  "[name]-[hash].[ext]",
]);

cpSync(path.join(packageRoot, "public/brand"), path.join(outputRoot, "brand"), { recursive: true });

const scriptName = findBuiltAsset(".js", true);
const stylesheetName = findBuiltAsset(".css", false);
const stylesheet = stylesheetName
  ? `<link rel="stylesheet" href="/console/assets/${stylesheetName}">`
  : "";

writeFileSync(
  path.join(outputRoot, "index.html"),
  [
    "<!doctype html>",
    '<html lang="en">',
    "<head>",
    '<meta charset="utf-8">',
    '<meta name="viewport" content="width=device-width, initial-scale=1">',
    "<title>Vifu Console</title>",
    stylesheet,
    "</head>",
    "<body>",
    '<div id="root"></div>',
    `<script type="module" src="/console/assets/${scriptName}"></script>`,
    "</body>",
    "</html>",
  ].filter(Boolean).join("\n"),
);

const html = readFileSync(path.join(outputRoot, "index.html"), "utf8");
if (!html.includes(`/console/assets/${scriptName}`)) {
  throw new Error("Embedded console index did not reference the JavaScript bundle.");
}

function findBuiltAsset(extension, required) {
  const matches = readdirSync(assetsDir)
    .filter((name) => name.startsWith("main-") && name.endsWith(extension))
    .sort();
  if (matches.length === 1) return matches[0];
  if (!required && matches.length === 0) return null;
  throw new Error(`Expected exactly one embedded console ${extension} asset, found ${matches.length}.`);
}

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: packageRoot,
    stdio: "inherit",
    env: process.env,
  });
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with status ${result.status ?? "unknown"}.`);
  }
}
