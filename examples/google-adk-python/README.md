# Google ADK With Vifu For Python

This example exposes an embedded Vifu endpoint as a Google ADK function tool.
ADK owns the outer conversation. Vifu owns the local provider, stable endpoint,
session state, Gateway connection, and trace for the delegated task.

## Run It

Create an environment from the repository root. Then install Vifu and Google
ADK. Use the constraints file that Google recommends for your Python version.

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install vifu google-adk
```

Start the ADK development CLI:

```bash
adk run examples/google-adk-python/vifu_adk_agent
```

Ask ADK to run a task on the local device. Replace `local_provider` in
`vifu_adk_agent/agent.py` with the model or framework used by your application.

The ADK model in this example selects when to call the tool. The provider
inside Vifu runs in the Python process. A local Vifu connection does not use a
pairing code.

See the [Google ADK integration guide](../../docs/integrations/google-adk.md)
for connection and production boundaries.
