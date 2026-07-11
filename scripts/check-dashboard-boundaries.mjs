import { access, readdir, readFile } from "node:fs/promises";
import path from "node:path";

const requiredDirectories = [
  "npm-packages/dashboard",
  "crates/vifu",
  "crates/vifu-server",
];
const sourceRoots = ["npm-packages/dashboard", "crates/vifu", "crates/vifu-server"];
const ignoredDirectories = new Set(["node_modules", ".next", ".next-e2e", "target"]);
const violations = [];
const dashboardForbiddenPatterns = [
  ["Cloudflare runtime package", /@opennextjs\/cloudflare/],
  ["Cloudflare request context", /getCloudflareContext/],
  ["private service binding", /\bAPI_GATEWAY\b/],
  ["deployment CLI configuration", /\bwrangler(?:\.jsonc?)?\b/i],
];

for (const directory of requiredDirectories) {
  if (!(await exists(directory))) violations.push(`${directory}: required directory is missing`);
}
await enforceDirectoryAllowlist("npm-packages", new Set(["dashboard"]));
await enforceDirectoryAllowlist("crates", new Set(["vifu", "vifu-server"]));

for (const root of sourceRoots) {
  for (const file of await sourceFiles(root)) {
    const contents = await readFile(file, "utf8");
    if (
      file.startsWith("npm-packages/dashboard/")
      && /process\.env\.VIFU_DEPLOYMENT_MODE/.test(contents)
    ) {
      violations.push(`${file}: dashboard authority must come from server capabilities`);
    }
    if (file.startsWith("npm-packages/dashboard/")) {
      for (const [label, pattern] of dashboardForbiddenPatterns) {
        if (pattern.test(contents)) violations.push(`${file}: ${label} is outside the public HTTP contract`);
      }
    }
  }
}

if (violations.length > 0) {
  console.error(violations.join("\n"));
  process.exit(1);
}

console.log("One capability-driven dashboard uses provider-neutral HTTP authority adapters.");

async function enforceDirectoryAllowlist(root, allowed) {
  for (const entry of await readdir(root, { withFileTypes: true })) {
    if (entry.isDirectory() && !allowed.has(entry.name)) {
      violations.push(`${path.join(root, entry.name)}: unexpected workspace directory`);
    }
  }
}

async function exists(target) {
  try {
    await access(target);
    return true;
  } catch {
    return false;
  }
}

async function sourceFiles(root) {
  const files = [];
  for (const entry of await readdir(root, { withFileTypes: true })) {
    if (entry.isDirectory() && ignoredDirectories.has(entry.name)) continue;
    const entryPath = path.join(root, entry.name);
    if (entry.isDirectory()) files.push(...await sourceFiles(entryPath));
    else if (/\.(?:rs|ts|tsx|js|mjs|json|toml)$/.test(entry.name)) files.push(entryPath);
  }
  return files;
}
