import { execFileSync } from "node:child_process";
import { rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const tsc = path.resolve(
  packageRoot,
  "..",
  "..",
  "node_modules",
  ".bin",
  process.platform === "win32" ? "tsc.cmd" : "tsc",
);

rmSync(path.join(packageRoot, "dist"), { recursive: true, force: true });
rmSync(path.join(packageRoot, "tsconfig.tsbuildinfo"), { force: true });

execFileSync(tsc, ["-p", "tsconfig.json"], {
  cwd: packageRoot,
  stdio: "inherit",
});
