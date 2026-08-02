import { copyFileSync, existsSync, mkdirSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const distRoot = path.join(packageRoot, "dist");
const tsc = path.resolve(
  packageRoot,
  "..",
  "..",
  "node_modules",
  ".bin",
  process.platform === "win32" ? "tsc.cmd" : "tsc",
);

rmSync(distRoot, { recursive: true, force: true });
rmSync(path.join(packageRoot, "tsconfig.tsbuildinfo"), { force: true });
execFileSync(tsc, ["-p", "tsconfig.json"], {
  cwd: packageRoot,
  stdio: "inherit",
});
mkdirSync(distRoot, { recursive: true });
const styles = path.join(packageRoot, "src", "styles.css");
if (existsSync(styles)) copyFileSync(styles, path.join(distRoot, "styles.css"));
