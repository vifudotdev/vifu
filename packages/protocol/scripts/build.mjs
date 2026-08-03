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
  "typescript",
  "bin",
  "tsc",
);

rmSync(path.join(packageRoot, "dist"), { recursive: true, force: true });
rmSync(path.join(packageRoot, "tsconfig.tsbuildinfo"), { force: true });

execFileSync(process.execPath, [tsc, "-p", "tsconfig.json"], {
  cwd: packageRoot,
  stdio: "inherit",
});
