# Web Research With Foundry Local and Vifu

This useful Agent App searches the web and produces a cited research brief with
a local Foundry model. Its `web-search` Agent obtains real sources. Its
`researcher` Agent synthesizes those sources and returns both the brief and
clickable source URLs.

Foundry Local still downloads, loads, serves, and unloads the model. The
program keeps its native client call and output assembly. Vifu adds the App,
two Agent identities, inference-stage timing, Device connection, sessions, and
Dashboard traces.

## Run It

Use Python 3.11 or a later version. Create an environment from the repository
root:

```bash
python3.11 -m venv .venv
. .venv/bin/activate
python -m pip install --upgrade "vifu[foundry]"
```

On Windows, install `"vifu[foundry-winml]"` for Windows ML acceleration.

Run the example:

```bash
python examples/foundry-local-python/main.py
```

The first run downloads the execution provider and the selected model. Later
runs use the local model cache.

The example searches for current uses of Arm-optimized on-device AI, sends the
returned sources to the local model, prints the cited brief, and lists every
source URL. Open the printed Dashboard URL and select **My Apps → Web
Research** to inspect both Agents and their separate traces.

`trace_foundry_stream` yields the original Foundry chunks and reports the
`first_token` and `decode` stages. It does not create a chat handler, manage
conversation history, or own the model lifecycle.

The example uses Bing's RSS search response so it needs network access for the
search step. Model inference remains inside Foundry Local.

Read the [Foundry Local integration guide](../../docs/integrations/foundry-local.md)
for integration details and platform notes.
