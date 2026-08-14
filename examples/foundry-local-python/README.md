# Foundry Local With Vifu For Python

This example registers a Foundry Local native chat client as a Vifu provider.
Model inference stays in the Python process. Vifu adds a stable endpoint,
session state, stage timing, output deltas, and optional Gateway monitoring.

## Run It

Create an environment from the repository root. Then install Vifu and the
Foundry Local package for your platform:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install vifu foundry-local-sdk
```

Use `foundry-local-sdk-winml` instead on supported Windows systems. Do not
install both variants in one environment.

Run the example:

```bash
python examples/foundry-local-python/main.py
```

The first run can download execution providers and the selected model. Later
inference uses the local model cache. The example reports `first_token` and
`decode` stages to Vifu.

See the [Foundry Local integration guide](../../docs/integrations/foundry-local.md)
to connect this Runtime to the Dashboard.
