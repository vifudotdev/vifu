import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = path.resolve(packageRoot, "..", "..");
const output = path.join(packageRoot, "dist");
const wasmOutput = path.join(output, "wasm");
const nativeOutput = path.join(output, "native");
const tsc = path.join(repositoryRoot, "node_modules", "typescript", "bin", "tsc");
const wasmInput = path.join(
  repositoryRoot,
  "target",
  "wasm32-unknown-unknown",
  "release",
  "vifu_wasm.wasm",
);

rmSync(output, { recursive: true, force: true });
mkdirSync(wasmOutput, { recursive: true });
mkdirSync(nativeOutput, { recursive: true });

execFileSync(
  "cargo",
  ["build", "--locked", "--release", "--target", "wasm32-unknown-unknown", "-p", "vifu-wasm"],
  { cwd: repositoryRoot, stdio: "inherit" },
);
execFileSync(
  "wasm-bindgen",
  [wasmInput, "--target", "nodejs", "--out-dir", wasmOutput, "--typescript"],
  { cwd: repositoryRoot, stdio: "inherit" },
);
execFileSync(
  "cargo",
  [
    "build",
    "--locked",
    "--release",
    "-p",
    "vifu-mobile-ffi",
    "--bin",
    "vifu-sdk-gateway",
    "--no-default-features",
  ],
  { cwd: repositoryRoot, stdio: "inherit" },
);
const gatewayFilename = process.platform === "win32" ? "vifu-sdk-gateway.exe" : "vifu-sdk-gateway";
const gatewayInput = path.join(repositoryRoot, "target", "release", gatewayFilename);
const gatewayOutput = path.join(nativeOutput, gatewayFilename);
copyFileSync(gatewayInput, gatewayOutput);
if (process.platform !== "win32") chmodSync(gatewayOutput, 0o755);
execFileSync(process.execPath, [tsc, "-p", "tsconfig.json"], {
  cwd: packageRoot,
  stdio: "inherit",
});
