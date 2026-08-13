import { access, readdir, readFile } from "node:fs/promises";
import path from "node:path";

const dashboardRoot = "npm-packages/dashboard";
const runtimeConsoleRoot = "packages/console";
const requiredDirectories = [dashboardRoot, runtimeConsoleRoot, "crates/vifu", "crates/vifu-server"];
const sourceRoots = [...requiredDirectories];
const cargoWorkspaceMembers = await readCargoWorkspaceMembers("Cargo.toml");
const workspaceCrateDirectories = new Set(
  cargoWorkspaceMembers
    .filter((member) => member.startsWith("crates/"))
    .map((member) => member.slice("crates/".length).split("/")[0])
    .filter(Boolean),
);
const ignoredDirectories = new Set(["node_modules", ".next", ".next-e2e", "target", "dist"]);
const violations = [];
const dashboardForbiddenPatterns = [
  ["edge-provider runtime package", /@opennextjs\//],
  ["edge-provider request context", /get[A-Z][A-Za-z]+Context/],
  ["host-specific service binding", /\bAPI_GATEWAY\b/],
  ["Dashboard database client", /from\s+["'](?:postgres|pg|@aws-sdk\/client-dynamodb)["']/],
  ["Dashboard password hashing", /\b(?:bcryptjs|argon2)\b/],
  ["hosted identity implementation", /(?:amazon-cognito|@aws-amplify\/auth|openidconnect)/],
  ["browser-visible credential", /NEXT_PUBLIC_[A-Z0-9_]*(?:KEY|TOKEN|SECRET|CREDENTIAL)/],
];
const runtimeConsoleForbiddenPatterns = [
  ["Dashboard host import", /npm-packages\/dashboard/],
  ["identity provider implementation", /(?:amazon-cognito|@aws-amplify\/auth|openidconnect)/],
  ["edge deployment implementation", /@opennextjs\//],
];
const serverForbiddenAuthPatterns = [
  ["web authentication route", /\/v1\/auth\/(?!exchange\b)/],
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
await enforceDirectoryAllowlist("npm-packages", new Set(["dashboard", "sdk"]));
await enforceDirectoryAllowlist("crates", workspaceCrateDirectories);

for (const root of sourceRoots) {
  for (const file of await sourceFiles(root)) {
    const contents = await readFile(file, "utf8");
    if (file.startsWith(`${dashboardRoot}/`)) {
      enforcePatterns(file, contents, dashboardForbiddenPatterns);
      if (/process\.env\.VIFU_DEPLOYMENT_MODE/.test(contents)) {
        violations.push(`${file}: runtime authority must come from server capabilities`);
      }
    }
    if (file.startsWith(`${runtimeConsoleRoot}/`)) {
      enforcePatterns(file, contents, runtimeConsoleForbiddenPatterns);
    }
    if (file.startsWith("crates/vifu-server/")) {
      enforcePatterns(file, contents, serverForbiddenAuthPatterns);
    }
  }
}

if (violations.length > 0) {
  console.error(violations.join("\n"));
  process.exit(1);
}

console.log("The Dashboard depends on the host-neutral @vifu/console boundary.");

function enforcePatterns(file, contents, patterns) {
  for (const [label, pattern] of patterns) {
    if (pattern.test(contents)) violations.push(`${file}: ${label} crosses the Dashboard boundary`);
  }
}

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
