# Build A Python Agent With Vifu

This tutorial embeds the native Vifu Runtime in a Python process, registers a
Python callable as an Agent, invokes it, and makes it ready for Dashboard
pairing.

The Python SDK is currently built from this repository. It uses generated
UniFFI bindings and the same Rust core as the mobile SDKs.

## 1. Build The SDK

From the Vifu repository root:

```bash
scripts/build-python-sdk.sh
export PYTHONPATH="$PWD/target/python-sdk"
```

## 2. Register An Agent

Create `app.py`:

```python
from vifu import AgentResponse, VifuRuntime

runtime = VifuRuntime("my-python-app")

def answer(request):
    prompt = request.input["prompt"]
    with request.trace.stage("decode", metadata={"model": "my-local-model"}):
        return AgentResponse(
            output={"text": f"Local answer: {prompt}"},
            metadata={"model": "my-local-model"},
        )

runtime.agent(
    "guide",
    answer,
    endpoint="chat",
    metadata={"model": "my-local-model"},
)
```

The Runtime stores session state and pending traces under
`~/.vifu/sdk/python/my-python-app` by default. Pass `data_dir=` to use an
application-owned directory.

## 3. Invoke The Endpoint

Add:

```python
result = runtime.invoke(
    "chat",
    {"prompt": "Explain the next step."},
    session_id="player-42",
)

print(result.output)
print(result.invocation_id)
print(result.trace)
```

Run it:

```bash
python3 app.py
```

The trace contains the completed provider stages followed by the total
`provider.invoke` stage. Long-running providers should call
`request.trace.activity()`. Streaming providers can call
`request.trace.output_delta(...)`.

## 4. Pair With The Dashboard

Start Vifu, open the Dashboard, select the App, and create a one-time device
pairing code. Pass that code on the first connection:

```python
gateway = runtime.connect(
    pairing_code,
    name="Python model on my laptop",
)
gateway.wait_until_connected()
```

The SDK stores the device identity and Server token with restricted file
permissions. On later starts, reconnect with `runtime.connect(name=...)` and
omit the consumed pairing code.

Content capture is off by default. Set `capture_trace_content=True` only after
the application has obtained user consent.

## 5. Let Python Manage A Local Server

An application can start the installed Vifu binary:

```python
from vifu import VifuServer

with VifuServer.start() as server:
    assert server.running
    # Start or pair application Runtimes here.
```

This is process management for the complete Vifu Server. It is not an HTTP
client wrapper.

## 6. Continue From A Working Example

Run [`examples/python-starter`](../../examples/python-starter/) first. Then use
the [Google ADK](../integrations/google-adk.md) or
[Foundry Local](../integrations/foundry-local.md) guides to replace the example
Provider with a real framework or on-device model.
