# Build a Python Agent With Vifu

This tutorial creates a Python Agent and connects it to a local Vifu Server.
The Server shows the Agent, Gateway, calls, and traces in its TUI and Dashboard.

## 1. Start Vifu

Download the Vifu archive for your computer. Extract it and start Vifu:

```bash
./vifu
```

Vifu creates one permanent Local app on the first start. Python Agents on this
computer join that App automatically.

## 2. Install the Python SDK

Create a virtual environment. Then install the prebuilt wheel:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install vifu
```

The wheel contains the Python API and the native Rust Runtime. The installation
does not compile Rust code.

## 3. Create an Agent

Create `app.py`:

```python
from vifu import AgentResponse, Vifu

app = Vifu(
    "my-python-agent",
    capture_trace_content=True,
)


@app.agent(
    "assistant",
    name="Python Assistant",
    metadata={"model": "python-example"},
)
def assistant(request):
    prompt = request.input["prompt"]
    with request.trace.stage("decode", metadata={"model": "python-example"}):
        return AgentResponse(
            output={"text": f"Hello from Python: {prompt}"},
            metadata={"model": "python-example"},
        )


result = app.invoke(
    "assistant",
    {"prompt": "Where did this Agent run?"},
)
print(result.output)

app.run()
```

The example enables trace-content capture for local development. Add a user
consent control before you enable this option in a distributed application.

## 4. Run the Agent

Run the Python process:

```bash
python app.py
```

The SDK completes these actions:

1. It opens the embedded Runtime.
2. It registers the provider, Agent, and endpoint.
3. It connects to `http://127.0.0.1:6790`.
4. It joins the permanent Local app.
5. It stores its Device Token for later starts.
6. It uploads traces and accepts remote calls.

The local path does not use a pairing code, App ID, API key, or configuration
file. Stop the Python process with `Ctrl+C`.

## 5. Inspect the Agent

Press `B` in the Vifu TUI. Open the Local app in the Dashboard.

The Gateway name is `Python: my-python-agent`. The Agent page shows the
`assistant` Agent. The trace shows the `decode` and `provider.invoke` stages.

## Use the Lower-Level API

Use `VifuRuntime` when the host application owns the lifecycle. Use
`VifuGateway` when a remote Server or a selected deployment requires explicit
enrollment.

See the [Google ADK](../integrations/google-adk.md) and
[Foundry Local](../integrations/foundry-local.md) guides for framework examples.
