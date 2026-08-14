# Build a Python Agent App With Vifu

This tutorial creates one real Vifu App with a Python Agent. The installed SDK
includes the Runtime, personal Server, Gateway, and Dashboard used during
development.

## 1. Install the Python SDK

Create a virtual environment. Then install the prebuilt wheel:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install vifu
```

The wheel contains the Python API, native Runtime, Agent Gateway, and local
Server. The installation uses prebuilt files for your platform.

## 2. Create an App and an Agent

Create `app.py`:

```python
from vifu import AgentResponse, Vifu

app = Vifu("workshop-guide", capture_trace_content=True)


@app.agent(
    "scene-planner",
    name="Python Scene Planner",
    capability="planning",
    metadata={"provider": "python-rules"},
)
def scene_planner(request):
    with request.trace.stage("validate", metadata={"provider": "python-rules"}):
        return AgentResponse(
            output={
                "action": "inspect",
                "target": request.input["scene"],
            },
            metadata={"provider": "python-rules"},
        )


def run_my_app(vifu):
    result = vifu.invoke(
        "scene-planner",
        {"scene": "workshop-door"},
        session_id="game-session-7",
    )
    print(result.output)


app.run(run_my_app)
```

The example enables trace-content capture for local development. Add a user
consent control before you enable this option in a distributed application.

## 3. Run the App

Run the Python process:

```bash
python app.py
```

The SDK completes these actions:

1. It reuses or starts your personal Server at `http://127.0.0.1:6790`.
2. It creates this App on its first run and records the binding in
   `.vifu/app.json`.
3. It opens the App's embedded Runtime.
4. It registers the provider, Agent, and endpoint.
5. It connects this Python process as a Device in that App.
6. It runs `run_my_app` and records each invocation.

The App ID is assigned by the Server. It is stable across later runs of this
project and visible in both `.vifu/app.json` and the Dashboard. Vifu does not
add a chat loop or prescribe an input and output schema. The registered handler
and `run_my_app` define the product.

## 4. Inspect the Agent

Open the Dashboard URL that the Python process prints.

The **My Apps** page now contains `workshop-guide`. Its Device is named
`Python: workshop-guide`. The Agents page shows the
`scene-planner` Agent. The trace shows the `validate` and `provider.invoke`
stages.

Create another directory with another `Vifu("name")` program to create another
App. Both Apps stay available in the same personal Dashboard and keep separate
Agents, sessions, endpoints, Devices, and traces.

## Let Your Application Own The Loop

Use the application as a context manager when an existing worker, API, game, or
test process owns the main loop:

```python
with app:
    result = app.invoke(
        "scene-planner",
        {"scene": "workshop-door"},
        session_id="game-session-7",
    )
    print(result.output)
```

Entering the context connects the Device. Leaving it closes resources owned by
this App process. The personal Server remains available to other Apps. The host
application remains responsible for its UI, input handling, and product
behavior.

## Use the Lower-Level API

Use `VifuRuntime` when the host application owns the lifecycle. Use
`VifuGateway` when a remote Server or a selected deployment requires explicit
enrollment. Use `VifuServer` when the host must control the Server process.

See the [Google ADK](../integrations/google-adk.md) and
[Foundry Local](../integrations/foundry-local.md) guides for framework examples.
