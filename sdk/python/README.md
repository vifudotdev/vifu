# Vifu Python SDK

The Python SDK embeds the Vifu Runtime, Agent Gateway, and local Server. Python
functions can become local Agent providers. `Vifu("name")` creates or reopens
one real App in the personal Server and records its project binding in
`.vifu/app.json`.

Create an environment and install Vifu:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install vifu
```

The wheel contains the generated UniFFI binding, native Rust library, and the
same complete Vifu binary distributed in Vifu releases. That binary includes
the Server, Dashboard, TUI, Gateway, SQLite storage, and Runtime APIs. The
Python API calls the Runtime in the application process. It reuses a running
local Server or starts the bundled binary in Server-only mode.

The public API provides:

- `Vifu` for App identity, Agent registration, local Server connection,
  invocation, tracing, and remote calls.
- `VifuRuntime` for persistent Runtime state, Agent registration, invocation,
  snapshots, and pending traces.
- `AgentTrace` for activity, output deltas, and typed provider stages.
- `VifuGateway` for explicit remote enrollment and advanced Gateway control.
- `VifuServer` for advanced control of the bundled Vifu Server process.
- `VifuServerConfig` for typed local Server startup configuration from Python.

Start with the [Python tutorial](../../docs/get-started/python.md). Runnable
framework integrations are available for
[Google ADK](../../examples/google-adk-python/) and
[Foundry Local](../../examples/foundry-local-python/).
