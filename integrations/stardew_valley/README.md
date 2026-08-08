# Run Stardew Valley with Vifu

This Stardew Valley demo uses the original StarDojo serial task loop and Mod as
its test harness, connected to one local Vifu llama provider. It is designed
for the Arm AI Optimization Challenge and for repeatable constrained-resource
work after the challenge.

The [Arm AI Optimization Challenge](https://arm-ai-optimization-challenge.devpost.com/)
submission deadline is August 14, 2026 at 4:00 PM PDT. The submission must show
an actual Arm optimization, so the demo pairs visible Stardew task completion
with controlled baseline/optimized measurements instead of treating an Arm run
alone as the result.

The integration pins:

- StarDojo commit `e251401cf1e84ba07cbfa08283a7aba52290e578`;
- `ggml-org/Qwen3-1.7B-GGUF` commit
  `daeb8e2d528a760970442092f6bf1e55c3b659eb`;
- `Qwen3-1.7B-Q4_K_M.gguf`, 1,282,439,264 bytes;
- SHA-256
  `d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5`.

[`roster.json`](roster.json) is the machine-readable authority for those pins,
the five demo tasks, the five task-suite counts, and the 5/21/50/95 roster
stages.

## What runs

```text
StarDojo task (serial)
  observe_v2 -> compact text observation -> pause game
       -> OpenAI-compatible project endpoint
       -> Vifu Agent Gateway
       -> one task Project Profile / model ID
       -> one shared resident Qwen3 1.7B model
  resume game -> execute StarDojo skill -> StarDojo evaluator
```

Vifu does not add a coordinator, scheduler, voting layer, or parallel game
runner. StarDojo still creates and evaluates one task runner at a time. The 95
Project Profiles keep task identity visible in the Console and traces while all
of them route to one physical llama Agent. Active task concurrency is one and
the model is loaded once.

## Prerequisites

- an Arm64 Mac for the challenge run;
- a user-owned Stardew Valley installation;
- [SMAPI](https://smapi.io/);
- StarDojoMod from the pinned StarDojo project;
- the pinned GGUF model above;
- a running Vifu Server and Console;
- the Python environment required by StarDojo.

StarDojo is MIT-licensed. Stardew Valley, SMAPI, saves, game assets, and the
GGUF model are external artifacts and are not vendored here.

## 1. Prepare the pinned StarDojo fork

```bash
cd integrations/stardew_valley/stardojo
git remote -v
git rev-parse HEAD
source setup.sh
```

This directory is the real fork of
`https://github.com/chenyanming/stardojo`, pinned to
`e251401cf1e84ba07cbfa08283a7aba52290e578`. The Vifu branch:

- points StarDojo's existing OpenAI-compatible provider at a Vifu project;
- maps each serial task to `stardew-valley-<suite>-<id>`;
- generates and stores the existing Basic skill embeddings through Vifu's
  OpenAI-compatible local embedding endpoint;
- sends text observations to the text-only model;
- adds strict JSON Schema actions and a shorter prompt in `optimized` mode;
- retains the original prompt and parser in `baseline` mode.

`source setup.sh` creates an isolated Python 3.11 `.venv`, installs the complete
pinned environment, and leaves the shell at the fork root. StarDojo remains the
owner of the Agent loop, skills, execution, and evaluation.

## 2. Start the Vifu binary and local Console

Install and start the normal Vifu application. Its default local configuration
runs Server and Agent Gateway together on `127.0.0.1:6790`:

```bash
cargo install vifu
vifu
```

Before starting it, add the demo's llama entry to `~/.vifu/providers.json` and
place the pinned model at the path configured by that entry. For a new Provider
registry, the included example expects this layout:

```bash
mkdir -p ~/.vifu/models
cp integrations/stardew_valley/providers.example.json ~/.vifu/providers.json
cp /path/to/Qwen3-1.7B-Q4_K_M.gguf ~/.vifu/models/
```

When the registry already contains Providers, merge only the
`stardew-valley-llama` entry instead of replacing the file. `modelPath` is
resolved relative to `providers.json`; an absolute path is also supported.

Configure the Console with the local Server URL and the same Admin Key used by
the binary in `npm-packages/dashboard/.env.local`, then start it:

```bash
cd npm-packages/dashboard
npm run dev
```

With the binary and Console running, bootstrap the Vifu Project from the
repository root:

```bash
python3 integrations/stardew_valley/harness.py bootstrap
set -a
. ./target/stardew_valley/.env.local
set +a
```

`bootstrap` reads the local admin credential and API URL from
`npm-packages/dashboard/.env.local` by default. Use `VIFU_ADMIN_KEY`,
`VIFU_API_BASE_URL`, or `--admin-env` when the Server and Console use another
local environment file.

The command is idempotent. It creates or reuses the `stardew_valley` Project
(API slug `stardew-valley`), selects the connected Gateway advertising
`stardew-valley-llama`, attaches or refreshes that Provider, and enables local
Gateway invocation on the primary `development` deployment. It then creates 95
logical task Profiles that all target the single physical llama Agent. Existing
Profiles are reused, and a changed Gateway route creates and activates only a
new Profile version.

Finally, it writes StarDojo-compatible runtime environment to
`target/stardew_valley/.env.local` with private file permissions. The file uses
the names StarDojo and the OpenAI Python SDK already understand:
`OA_OPENAI_KEY` for the Project key and `OPENAI_BASE_URL` for the Project API
URL. If `STARDEW_APP_PATH` is already present in the shell or existing env file,
bootstrap preserves it. The Admin Key remains in the Console environment.

Open `http://localhost:6791/project/stardew-valley` to inspect the Project,
Agents, endpoints, and traces while the demo runs.

## 3. Run the doctor

From the Vifu repository:

```bash
python3 integrations/stardew_valley/harness.py doctor \
  --stardojo /path/to/stardojo \
  --providers target/stardew_valley/providers.json \
  --server-url http://127.0.0.1:6790
```

The doctor fails when the host architecture, upstream commit, StarDojo files,
SMAPI executable, StarDojoMod manifest, model size/hash, or Vifu health check
does not match the reproducible setup.

## 4. Prove the Agent roster

Load `target/stardew_valley/.env.local` as shown above. Each stage sends a real
structured model invocation to every selected endpoint and writes JSONL
evidence under `target/stardew_valley/`:

```bash
python3 integrations/stardew_valley/harness.py smoke --stage 5
python3 integrations/stardew_valley/harness.py smoke --stage 21
python3 integrations/stardew_valley/harness.py smoke --stage 50
python3 integrations/stardew_valley/harness.py smoke --stage 95
```

The first five are the fixed demo board. The first 21 cover all Farming Lite
task Agents. A stage passes only when every endpoint returns schema-valid JSON
and OpenAI-compatible usage data.

## 5. Run the fixed Stardew demo

Prepare StarDojo's Python environment from the embedded fork:

```bash
cd integrations/stardew_valley/stardojo
source setup.sh
```

Start the pinned prebuilt Mod on its default port in one terminal; it processes
only the first recognized startup option, so the screenshot setting must come
first:

```bash
export STARDEW_APP_PATH="/path/to/StardewModdingAPI"
"$STARDEW_APP_PATH" --sample-rate 0
```

After SMAPI reports `Mods loaded and ready`, run from the fork root in a second
terminal:

```bash
set -a
. /path/to/vifu/target/stardew_valley/.env.local
set +a
python env/llm_env_multi_tasks.py --port 10783
```

The Vifu path uses text observations, so the overlay passes
`--sample-rate 0` to StardojoMod. This keeps the state/action bridge active and
disables unused backbuffer capture, including the Retina-size mismatch present
in the pinned macOS Mod build. Port `10783` is the Mod's built-in default and is
also the Vifu overlay default. Set a non-zero sample rate only when testing an
image consumer separately.

The default board uses official Farming Lite task IDs `0`, `1`, `3`, `6`, and
`8`: clear weeds, clear stones, till tiles, sow cauliflower seeds, and water
crops.

For the controlled baseline, keep the model, task board, save state, seed,
temperature, context size, and hardware fixed, then run:

```bash
export VIFU_STARDEW_VALLEY_MODE=baseline
python env/llm_env_multi_tasks.py
```

Use the evidence protocol in [Performance](../../docs/performance.md) before
publishing a comparison.

## Files

| File | Purpose |
| --- | --- |
| [`roster.json`](roster.json) | Upstream/model pins, task counts, stages, and demo board |
| [`providers.example.json`](providers.example.json) | Local llama model and resource settings for `providers.json` |
| [`stardojo/`](stardojo/) | Pinned StarDojo fork with the Vifu provider integration |
| [`harness.py`](harness.py) | Project bootstrap, doctor, private identity generation, and staged real invocations |
| [`test_harness.py`](test_harness.py) | Idempotence, route-refresh, and private-environment regression tests |
