import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = path.resolve(packageRoot, "..", "..");
const vitest = path.join(repositoryRoot, "node_modules", "vitest", "vitest.mjs");

execFileSync(process.execPath, [path.join(packageRoot, "scripts", "build.mjs")], {
  cwd: packageRoot,
  stdio: "inherit",
});
execFileSync(
  process.execPath,
  [vitest, "run", "npm-packages/sdk/test/gateway.test.ts"],
  { cwd: repositoryRoot, stdio: "inherit" },
);
