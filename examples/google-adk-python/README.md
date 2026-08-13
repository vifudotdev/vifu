# Google ADK With Vifu For Python

This example exposes an embedded Vifu endpoint as a Google ADK function tool.
ADK owns the outer conversation. Vifu owns the local provider, stable endpoint,
session state, Gateway connection, and trace for the delegated task.

## Run It

From the repository root, build the Vifu Python SDK:

```bash
scripts/build-python-sdk.sh
```

Create an environment for this example and install Google ADK. Use the
constraints file recommended by Google for your Python version.

```bash
python3 -m venv .venv
. .venv/bin/activate
pip install google-adk
```

Make the generated Vifu package visible, then start the ADK development CLI:

```bash
export PYTHONPATH="$PWD/target/python-sdk"
adk run examples/google-adk-python/vifu_adk_agent
```

Ask ADK to run a task on the local device. Replace `local_provider` in
`vifu_adk_agent/agent.py` with the model or framework used by your application.

The ADK model in this example selects when to call the tool. The provider
inside Vifu runs in the Python process. Pair the Runtime with a Vifu Server if
you also want its Vifu trace in the Dashboard.

See the [Google ADK integration guide](../../docs/integrations/google-adk.md)
for pairing and production boundaries.
