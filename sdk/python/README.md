# Vifu Python SDK

The Python SDK embeds the Vifu Runtime, Agent Gateway, and local Server. Python
functions can become local Agent providers. `Vifu.run()` manages the complete
local lifecycle and opens the same Dashboard and tracing path as the Vifu CLI.

Create an environment and install Vifu:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install vifu
```

The wheel contains the generated UniFFI binding, native Rust library, and Vifu
Server binary. The Python API calls the Runtime in the application process. It
reuses a running local Server or starts its bundled Server.

The public API provides:

- `Vifu` for Agent registration, local Server connection, invocation, tracing,
  and remote calls.
- `VifuRuntime` for persistent Runtime state, Agent registration, invocation,
  snapshots, and pending traces.
- `AgentTrace` for activity, output deltas, and typed provider stages.
- `VifuGateway` for explicit remote enrollment and advanced Gateway control.
- `VifuServer` for advanced control of the bundled Vifu Server process.

Start with the [Python tutorial](../../docs/get-started/python.md). Runnable
framework integrations are available for
[Google ADK](../../examples/google-adk-python/) and
[Foundry Local](../../examples/foundry-local-python/).
