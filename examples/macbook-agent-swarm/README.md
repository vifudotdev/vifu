# MacBook agent swarm

`swarm.py` is a standard-library Python benchmark for a local Vifu Gateway on
an Apple-silicon MacBook. It creates a dedicated Vifu project, gives many
logical agents routes to the same resident Provider resource, then measures
their concurrent chat requests. This makes queueing, latency, and aggregate
token throughput visible while you tune a local GGUF configuration.

The benchmark never changes `~/.vifu/providers.json`, does not require a
Python package install, and never writes credentials to its report. It creates
a least-privilege Project API key for one run and deletes that key afterwards.

## Prerequisites

1. Install or build the Vifu CLI, and configure one local llama Provider. The
   [local llama guide](../../providers/llama/README.md) has the complete
   `providers.json` schema. This example uses `local-qwen` below as a sample
   Provider key only.
2. Make the local Server administrator credential available as
   `VIFU_ADMIN_KEY` in the shell environments that start Vifu and this script.
   The script uses it only to set up its benchmark project; it never logs it.
3. Use Python 3.9 or later. No third-party packages are needed.

For a first run, keep the Provider's `maxConcurrency` at `1`. The many logical
agents will then reveal the queue introduced by one resident model. Change one
Provider setting at a time and retain the reports for comparison.

## Run

In terminal A, start the Vifu CLI and leave it running:

```bash
vifu
```

In terminal B, configure the benchmark project and run eight logical agents,
with three sequential requests from each agent and at most eight agents active
at once:

```bash
python3 examples/macbook-agent-swarm/swarm.py \
  --provider-key local-qwen \
  --agents 8 \
  --requests-per-agent 3 \
  --max-in-flight 8
```

The script waits up to 60 seconds for the Gateway to advertise the named
Provider. Use `--setup-wait-seconds 0` when a model host needs an unbounded
startup wait. The Python client intentionally has no default request deadline;
each route is configured with Vifu's supported 120-second runtime timeout.
Use `--request-timeout-seconds` only when the caller itself needs a stricter
limit.

The JSON report is written to:

```text
~/Library/Caches/Vifu/macbook-agent-swarm.json
```

An adjacent self-contained HTML/SVG chart is generated automatically:

```text
~/Library/Caches/Vifu/macbook-agent-swarm.html
```

Open the HTML file locally to inspect the summary cards and per-request latency
bars. It has no Python dependency or network dependency. `--chart PATH` writes
it elsewhere and `--no-chart` keeps the JSON-only behavior.

The JSON and chart contain device metadata, benchmark settings, percentile
latency, aggregate completion throughput, and per-request status. They
deliberately omit prompt and completion content and all credentials.

## Compare a provider setting

Keep the command constant, edit only one setting in `~/.vifu/providers.json`,
then let Vifu reconnect and run the same command again. For example, compare
`contextSize`, `defaultMaxTokens`, or a supported `maxConcurrency` value. The
most useful values to compare are:

- `summary.wallTimeMs` for total swarm completion time.
- `summary.latencyMs.p95` for the slowest logical agents under load.
- `summary.completionTokensPerSecond` for aggregate completion throughput.
- `summary.failed` before increasing concurrency further.

Logical agents share one physical Provider resource on purpose. Increasing
`--agents` or `--max-in-flight` tests Vifu's request scheduling and the
MacBook's resident-model behavior; it does not duplicate the GGUF in memory.

## Clean up

The project and logical routes are intentionally reused by future runs under
the default `macbook-agent-swarm` slug. To isolate an experiment, supply a
different `--project-slug`. The temporary Project API key is deleted by the
script at the end of every completed run.
