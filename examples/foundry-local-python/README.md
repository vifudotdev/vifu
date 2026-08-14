# Web Research With Foundry Local and Vifu

This conversational Agent App accepts a research topic, searches the web, and
produces a cited brief with a local Foundry model. Enter another topic to start
a new research session. Its `web-search` Agent obtains real sources. Its
`researcher` Agent synthesizes those sources and returns both the brief and
clickable source URLs.

Foundry Local still downloads, loads, serves, and unloads the model. The
program keeps its native client call and output assembly. Vifu adds the App,
two Agent identities, inference-stage timing, Device connection, sessions, and
Dashboard traces. The installed Vifu wheel contains the complete Vifu binary,
including its embedded Dashboard; the example starts it automatically when a
local Server is not already running.

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

At the `Research>` prompt, enter any topic:

```text
Research> What changed in Arm on-device AI this month?
```

For each topic, the example searches an English-language current-news feed,
sends the returned sources to the local model, prints a source-constrained
draft, and lists every numbered source URL. Each topic gets its own Vifu
session. Open the printed Dashboard URL and select **My Apps → Web Research**
to inspect both Agents and their traces. Open **Agents → Local Researcher →
Prompt** to inspect or edit the source-constrained research prompt. Select
**Save & make live**; the next topic uses the new instructions through
`request.instructions`. Enter `exit` or `quit`, or press `Ctrl+C`, to stop the
App.

The search Agent removes conversational wrappers and quotation marks before
searching. If the current-news feed has no results, it automatically retries
with general web search. The terminal prints a rewritten query whenever it
differs from the user's input.

`trace_foundry_stream` yields the original Foundry chunks and reports the
`first_token` and `decode` stages. It does not create a chat handler, manage
conversation history, or own the model lifecycle.

The example uses Google News RSS for the current-research step and Bing RSS for
general searches, so search needs network access. Model inference remains
inside Foundry Local.

Read the [Foundry Local integration guide](../../docs/integrations/foundry-local.md)
for integration details and platform notes.
