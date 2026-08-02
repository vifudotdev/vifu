import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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
  "[name].[ext]",
  "--asset-naming",
  "[name]-[hash].[ext]",
]);

cpSync(path.join(packageRoot, "public/brand"), path.join(outputRoot, "brand"), { recursive: true });

const stylesheet = existsSync(path.join(assetsDir, "main.css"))
  ? '<link rel="stylesheet" href="/console/assets/main.css">'
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
    '<script type="module" src="/console/assets/main.js"></script>',
    "</body>",
    "</html>",
  ].filter(Boolean).join("\n"),
);

const html = readFileSync(path.join(outputRoot, "index.html"), "utf8");
if (!html.includes("/console/assets/main.js")) {
  throw new Error("Embedded console index did not reference the JavaScript bundle.");
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
