# Build a Python Agent With Vifu

This tutorial creates a Python Agent with terminal chat and a local Dashboard.
The Dashboard shows the Agent, Gateway, calls, and traces.

## 1. Install the Python SDK

Create a virtual environment. Then install the prebuilt wheel:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install vifu
```

The wheel contains the Python API, native Runtime, Agent Gateway, and local
Server. The installation uses prebuilt files for your platform.

## 2. Create an Agent

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


app.run()
```

The example enables trace-content capture for local development. Add a user
consent control before you enable this option in a distributed application.

## 3. Run the Agent

Run the Python process:

```bash
python app.py
```

The SDK completes these actions:

1. It opens the embedded Runtime.
2. It registers the provider, Agent, and endpoint.
3. It reuses or starts the Server at `http://127.0.0.1:6790`.
4. It connects the Agent Gateway to the permanent Local app.
5. It stores its Device Token for later starts.
6. It serves terminal prompts and remote endpoint calls.
7. It sends each trace to the local Dashboard.

Enter a prompt at `You >`. Enter `/quit` or press `Ctrl+C` to stop the process.

## 4. Inspect the Agent

Open the Dashboard URL that the Python process prints.

The Gateway name is `Python: my-python-agent`. The Agent page shows the
`assistant` Agent. The trace shows the `decode` and `provider.invoke` stages.

## Use the Lower-Level API

Use `VifuRuntime` when the host application owns the lifecycle. Use
`VifuGateway` when a remote Server or a selected deployment requires explicit
enrollment. Use `VifuServer` when the host must control the Server process.

See the [Google ADK](../integrations/google-adk.md) and
[Foundry Local](../integrations/foundry-local.md) guides for framework examples.
