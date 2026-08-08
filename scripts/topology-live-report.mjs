import { readFile, writeFile } from "node:fs/promises";
import process from "node:process";

const [resultsPath, jsonPath, junitPath, startedAt, finishedAt] = process.argv.slice(2);
if (!resultsPath || !jsonPath || !junitPath || !startedAt || !finishedAt) {
  throw new Error(
    "usage: topology-live-report.mjs RESULTS REPORT_JSON JUNIT_XML STARTED_AT FINISHED_AT",
  );
}

const rows = (await readFile(resultsPath, "utf8"))
  .split("\n")
  .filter(Boolean)
  .map((line) => {
    const [name, packageName, test, status, durationSeconds, log] = line.split("\t");
    return {
      name,
      package: packageName,
      test,
      status,
      durationMs: Number(durationSeconds) * 1_000,
      log,
    };
  });
const passed = rows.filter((row) => row.status === "passed").length;
const failed = rows.length - passed;
const durationMs = rows.reduce((sum, row) => sum + row.durationMs, 0);
const report = {
  schemaVersion: 1,
  suite: "vifu-topology-live",
  startedAt,
  finishedAt,
  durationMs,
  environment: {
    runtimeState: "isolated-per-case",
    serverDatabase: "temporary-sqlite",
    network: "loopback-random-port",
    dockerRequired: false,
  },
  summary: {
    total: rows.length,
    passed,
    failed,
  },
  cases: rows,
};
await writeFile(jsonPath, `${JSON.stringify(report, null, 2)}\n`);

const xml = (value) =>
  String(value)
    .replaceAll("&", "&amp;")
    .replaceAll('"', "&quot;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
const testCases = rows
  .map((row) => {
    const failure =
      row.status === "passed"
        ? ""
        : `<failure message="topology live test failed">See ${xml(row.log)}</failure>`;
    return `  <testcase classname="${xml(row.package)}" name="${xml(row.name)}" time="${(
      row.durationMs / 1_000
    ).toFixed(3)}">${failure}</testcase>`;
  })
  .join("\n");
const junit = `<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="vifu-topology-live" tests="${rows.length}" failures="${failed}" time="${(
  durationMs / 1_000
).toFixed(3)}">
${testCases}
</testsuite>
`;
await writeFile(junitPath, junit);
