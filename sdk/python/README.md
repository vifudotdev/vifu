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
- `LocalWhisper` and `LocalLlama` for code-configured, in-process speech and
  language-model Providers.
- `Vifu.run()` for the same registered Agent App lifecycle locally and on a
  compatible managed host.

Call `app.run()` after registering Agents to serve Endpoint invocations until
the local process stops. An App may instead pass a local entrypoint such as
`app.run(local_main)`. Vifu calls that entrypoint for direct local execution. A
compatible managed host uses the registered Endpoints from the same script,
does not enter the local interaction loop, and stops after its assigned
invocation completes. Managed lifecycle details are not part of application
input or output.

Framework integrations can be registered as normal Agent handlers. An
integration may declare `vifu_name`, `vifu_capability`, `vifu_metadata`,
`vifu_instructions`, and related defaults on the handler object. It may also
implement `vifu_bind(...)` and `vifu_run()` when it owns one foreground runtime
such as a voice session. `Vifu.run()` binds and starts that lifecycle, so the
application entrypoint remains only Agent registration:

```python
app.agent("voice", voice_agent)
app.agent("assistant", assistant_agent)
app.run()
```

Install the optional Strands integration, then select an OpenAI-compatible
Provider directly in Python, including hosted and loopback model servers:

```bash
python -m pip install "vifu[strands]"
```

```python
from vifu.integrations.strands import openai_compatible_model

model = openai_compatible_model(
    request,
    url="http://127.0.0.1:11434/v1/chat/completions",
    model="qwen2.5:7b",
)
```

The API key remains process configuration and is never included in Agent
metadata. `strands_model(...)` remains available when an App intentionally uses
a versioned Vifu Agent Profile.

Start with the [Python tutorial](../../docs/get-started/python.md). Runnable
framework integrations are available for
[Google ADK](../../examples/google-adk-python/) and
[Foundry Local](../../examples/foundry-local-python/).
