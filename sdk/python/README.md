# Vifu Python SDK

The Python SDK embeds Vifu Runtime and Agent Gateway through the same Rust core
used by the mobile SDKs. Python callables become local providers. They can run
directly in the process or join a Vifu App through Gateway pairing.

Create an environment and install Vifu:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install vifu
```

The wheel contains the generated UniFFI binding and the native Rust library.
The Python API calls that library in the application process.

The public API provides:

- `VifuRuntime` for persistent Runtime state, Agent registration, invocation,
  snapshots, and pending traces.
- `AgentTrace` for activity, output deltas, and typed provider stages.
- `VifuGateway` for one-time pairing, stored identity, reconnect, remote
  invocation, and Dashboard telemetry.
- `VifuServer` for managing the complete installed Vifu process.

Start with the [Python tutorial](../../docs/get-started/python.md). Runnable
framework integrations are available for
[Google ADK](../../examples/google-adk-python/) and
[Foundry Local](../../examples/foundry-local-python/).
