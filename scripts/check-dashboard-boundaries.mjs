import { access, readdir, readFile } from "node:fs/promises";
import path from "node:path";

const requiredDirectories = [
  "npm-packages/dashboard",
  "crates/vifu",
  "crates/vifu-server",
];
const sourceRoots = ["npm-packages/dashboard", "crates/vifu", "crates/vifu-server"];
const cargoWorkspaceMembers = await readCargoWorkspaceMembers("Cargo.toml");
const workspaceCrateDirectories = new Set(
  cargoWorkspaceMembers
    .filter((member) => member.startsWith("crates/"))
    .map((member) => member.slice("crates/".length).split("/")[0])
    .filter(Boolean),
);
const ignoredDirectories = new Set(["node_modules", ".next", ".next-e2e", "target"]);
const violations = [];
const dashboardForbiddenPatterns = [
  ["edge-provider runtime package", /@opennextjs\//],
  ["edge-provider request context", /get[A-Z][A-Za-z]+Context/],
  ["private service binding", /\bAPI_GATEWAY\b/],
];
const serverForbiddenAuthPatterns = [
  ["web authentication route", /\/v1\/auth\//],
  ["Dashboard auth environment", /\bVIFU_AUTH_/],
  ["password hashing dependency", /\bargon2\b/],
  ["OIDC dependency", /\bopenidconnect\b/],
];

for (const directory of requiredDirectories) {
  if (!(await exists(directory))) violations.push(`${directory}: required directory is missing`);
}
for (const directory of sourceRoots.filter((root) => root.startsWith("crates/"))) {
  if (!cargoWorkspaceMembers.includes(directory)) {
    violations.push(`${directory}: required crate must be listed in Cargo workspace members`);
  }
}
if (workspaceCrateDirectories.size === 0) {
  violations.push("Cargo.toml: workspace members must include at least one crates/* entry");
}
await enforceDirectoryAllowlist("npm-packages", new Set(["dashboard"]));
await enforceDirectoryAllowlist("crates", workspaceCrateDirectories);

for (const root of sourceRoots) {
  for (const file of await sourceFiles(root)) {
    const contents = await readFile(file, "utf8");
    if (
      file.startsWith("npm-packages/dashboard/")
      && file !== "npm-packages/dashboard/lib/dashboard-auth-config.ts"
      && /process\.env\.VIFU_DEPLOYMENT_MODE/.test(contents)
    ) {
      violations.push(`${file}: runtime authority must come from server capabilities; auth provider selection belongs in dashboard-auth-config.ts`);
    }
    if (file.startsWith("npm-packages/dashboard/")) {
      for (const [label, pattern] of dashboardForbiddenPatterns) {
        if (pattern.test(contents)) violations.push(`${file}: ${label} is outside the public HTTP contract`);
      }
    }
    if (file.startsWith("crates/vifu-server/")) {
      for (const [label, pattern] of serverForbiddenAuthPatterns) {
        if (pattern.test(contents)) violations.push(`${file}: ${label} belongs to the Dashboard, not vifu-server`);
      }
    }
  }
}

if (violations.length > 0) {
  console.error(violations.join("\n"));
  process.exit(1);
}

console.log("One dashboard uses provider-neutral runtime adapters and Next-owned auth provider configuration.");

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

async function readCargoWorkspaceMembers(target) {
  const contents = await readFile(target, "utf8");
  const lines = contents.split(/\r?\n/);
  let inWorkspace = false;
  let inMembers = false;
  let membersBlock = "";

  for (const line of lines) {
    if (/^\s*\[/.test(line)) {
      if (inWorkspace) break;
      inWorkspace = /^\s*\[workspace\]\s*$/.test(line);
      continue;
    }
    if (!inWorkspace) continue;

    if (!inMembers) {
      const start = line.match(/^\s*members\s*=\s*\[(.*)$/);
      if (!start) continue;
      inMembers = true;
      membersBlock += `${start[1]}\n`;
      if (start[1].includes("]")) break;
      continue;
    }

    membersBlock += `${line}\n`;
    if (line.includes("]")) break;
  }

  return Array.from(membersBlock.matchAll(/"([^"]+)"/g), ([, member]) => member);
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
