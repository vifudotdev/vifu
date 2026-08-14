# Vifu Python SDK

The Python SDK embeds the Vifu Runtime and Agent Gateway. Python functions can
become local Agent providers. The default API connects them to a local Vifu
Server and its permanent Local app.

Create an environment and install Vifu:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install vifu
```

The wheel contains the generated UniFFI binding and the native Rust library.
The Python API calls that library in the application process.

The public API provides:

- `Vifu` for Agent registration, local Server connection, invocation, tracing,
  and remote calls.
- `VifuRuntime` for persistent Runtime state, Agent registration, invocation,
  snapshots, and pending traces.
- `AgentTrace` for activity, output deltas, and typed provider stages.
- `VifuGateway` for explicit remote enrollment and advanced Gateway control.
- `VifuServer` for managing the complete installed Vifu process.

Start with the [Python tutorial](../../docs/get-started/python.md). Runnable
framework integrations are available for
[Google ADK](../../examples/google-adk-python/) and
[Foundry Local](../../examples/foundry-local-python/).
