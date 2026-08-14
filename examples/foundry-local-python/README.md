# Foundry Local With Vifu For Python

This example runs an interactive Foundry Local chat Agent. Foundry Local runs
the model in the Python process. Vifu supplies the endpoint, session state,
stage timing, output deltas, Gateway connection, and Dashboard traces.

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

Enter prompts at the `You >` prompt. Enter `/quit` to stop the Agent. Open the
printed Dashboard URL to inspect the Gateway, Agent, and traces.

The example keeps one Vifu session for the terminal conversation. It reports
the `first_token` and `decode` stages for each response.

Read the [Foundry Local integration guide](../../docs/integrations/foundry-local.md)
for integration details and platform notes.
