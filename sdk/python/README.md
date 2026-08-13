# Vifu Python SDK

The Python SDK embeds Vifu Runtime and Agent Gateway through the same Rust core
used by the mobile SDKs. Python callables become local providers. They can run
directly in the process or join a Vifu App through Gateway pairing.

Build and run the starter from the repository root:

```bash
scripts/build-python-sdk.sh
PYTHONPATH=target/python-sdk python3 examples/python-starter/main.py
```

The build creates generated UniFFI bindings and a native library under
`target/python-sdk`.

The public API provides:

- `VifuRuntime` for persistent Runtime state, Agent registration, invocation,
  snapshots, and pending traces;
- `AgentTrace` for activity, output deltas, and typed provider stages;
- `VifuGateway` for one-time pairing, stored identity, reconnect, remote
  invocation, and Dashboard telemetry;
- `VifuServer` for managing the complete installed Vifu process.

Start with the [Python tutorial](../../docs/get-started/python.md). Runnable
framework integrations are available for
[Google ADK](../../examples/google-adk-python/) and
[Foundry Local](../../examples/foundry-local-python/).

The source build is the supported path in this revision. A future wheel must
package the generated module and native library for each target platform before
it is advertised as a binary install.
