import { readFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";

const checks = [
  ["provider access key", /(?:AKIA|ASIA)[0-9A-Z]{16}/],
  ["source-control token", /(?:github_pat_|gh[pousr]_)[A-Za-z0-9_]+/],
  ["API key", /\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}/],
  ["infrastructure account ID", /\b\d{12}\b/],
  ["private key", /-----BEGIN (?:RSA |OPENSSH |EC |DSA )?PRIVATE KEY-----/],
];
const privateTerms = (process.env.VIFU_PRIVATE_TERMS ?? "")
  .split(/\r?\n/)
  .map((term) => term.trim().toLocaleLowerCase())
  .filter(Boolean);
privateTerms.push(["con", "senger"].join(""));
const historyPattern = [
  "(AKIA|ASIA)[0-9A-Z]{16}",
  "github_pat_[A-Za-z0-9_]+",
  "gh[pousr]_[A-Za-z0-9_]+",
  "sk-(proj-)?[A-Za-z0-9_-]{20,}",
  "\\b[0-9]{12}\\b",
  "BEGIN (RSA |OPENSSH |EC |DSA )?PRIVATE KEY",
].join("|");
const violations = [];
const scannerPath = path.normalize("scripts/check-public-repo.mjs");
const generatedOutputPattern = new RegExp(
  `(^|/)(?:${[
    "\\.next",
    "\\.next-e2e",
    `\\.${["open", "next"].join("-")}`,
    `\\.${["wrang", "ler"].join("")}`,
    "dist",
    "test-results",
    "playwright-report",
  ].join("|")})(/|$)`,
);

const sourceFiles = git(["ls-files", "--cached", "--others", "--exclude-standard", "-z"])
  .split("\0")
  .filter(Boolean);
for (const file of sourceFiles) {
  const normalized = path.normalize(file);
  if (normalized === path.normalize("AGENTS.md") || normalized === scannerPath) continue;
  if (generatedOutputPattern.test(file)) continue;
  const contents = await readFile(file, "utf8").catch(() => "");
  for (const [label, pattern] of checks) {
    if (pattern.test(contents)) violations.push(`${file}: ${label}`);
  }
  const normalizedContents = contents.toLocaleLowerCase();
  if (privateTerms.some((term) => normalizedContents.includes(term))) {
    violations.push(`${file}: private denylist match`);
  }
}

const tracked = git(["ls-files", "-z"]).split("\0").filter(Boolean);
for (const file of tracked) {
  if (file === "AGENTS.md") violations.push("AGENTS.md: internal operating notes are tracked");
  if (generatedOutputPattern.test(file)) {
    violations.push(`${file}: generated output is tracked`);
  }
  if (/(^|\/)\.env(?:\.|$)/.test(file) && path.basename(file) !== ".env.example") {
    violations.push(`${file}: local environment file is tracked`);
  }
  if (/(^|\/)(?:screenshot|screen-shot|artifacts?)(?:\/|[-_.])/i.test(file)) {
    violations.push(`${file}: internal screenshot or artifact is tracked`);
  }
}

const agentHistory = git(["log", "--all", "--format=", "--name-only", "--", "AGENTS.md"]);
if (agentHistory.trim()) violations.push("AGENTS.md: internal operating notes exist in Git history");

const revisions = git(["rev-list", "--all"]).trim().split("\n").filter(Boolean);
for (const revision of revisions) {
  const result = spawnSync(
    "git",
    [
      "grep",
      "-I",
      "-n",
      "-E",
      historyPattern,
      revision,
      "--",
      ".",
      ":(exclude)Cargo.lock",
      ":(exclude).github/workflows/*",
      ":(exclude)scripts/check-public-repo.mjs",
    ],
    { encoding: "utf8", maxBuffer: 8 * 1024 * 1024 },
  );
  if (result.status === 0 && result.stdout.trim()) {
    const matches = result.stdout.trim().split("\n").slice(0, 5);
    violations.push(...matches.map((match) => `${revision.slice(0, 12)}:${match}: public-history violation`));
  } else if (result.status !== 1) {
    violations.push(`Could not scan Git revision ${revision.slice(0, 12)}.`);
  }

  for (const term of privateTerms) {
    const privateResult = spawnSync(
      "git",
      ["grep", "-I", "-n", "-i", "-F", term, revision, "--", "."],
      { encoding: "utf8", maxBuffer: 8 * 1024 * 1024 },
    );
    if (privateResult.status === 0 && privateResult.stdout.trim()) {
      violations.push(`${revision.slice(0, 12)}: private-history denylist match`);
    } else if (privateResult.status !== 1) {
      violations.push(`Could not scan private terms in Git revision ${revision.slice(0, 12)}.`);
    }
  }
}

if (violations.length > 0) {
  console.error([...new Set(violations)].join("\n"));
  process.exit(1);
}

console.log("Public files, tracked output, and Git history passed hygiene checks.");

function git(args) {
  const result = spawnSync("git", args, {
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
  if (result.status !== 0) {
    throw new Error(`git ${args.join(" ")} failed: ${result.stderr.trim()}`);
  }
  return result.stdout;
}
