# Build a Python Agent App With Vifu

This tutorial creates one real Vifu App with a Python Agent. The installed SDK
includes the Runtime, personal Server, Gateway, and Dashboard used during
development.

## 1. Install the Python SDK

Create a project and add the prebuilt wheel with
[`uv`](https://docs.astral.sh/uv/getting-started/installation/):

```bash
uv init --python 3.11
uv add vifu
```

The wheel contains the Python API, native Runtime, Agent Gateway, and local
Server. The installation uses prebuilt files for your platform.

## 2. Create an App and an Agent

Create `app.py`:

```python
import time

from vifu import AgentResponse, Vifu

app = Vifu("workshop-guide", capture_trace_content=True)


@app.agent(
    "scene-planner",
    name="Python Scene Planner",
    capability="planning",
    metadata={"provider": "python-rules"},
    instructions="Inspect the scene and return the safest next action.",
)
def scene_planner(request):
    with request.trace.stage("validate", metadata={"provider": "python-rules"}):
        return AgentResponse(
            output={
                "action": "inspect",
                "target": request.input["scene"],
                "guidance": request.instructions,
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
    print("App and Agents are online. Press Ctrl+C to stop.")
    while True:
        time.sleep(3_600)


app.run(run_my_app)
```

The example enables trace-content capture for local development. Add a user
consent control before you enable this option in a distributed application.

## 3. Run the App

Run the Python process:

```bash
uv run app.py
```

The SDK completes these actions:

1. It reuses or starts your personal Server at `http://127.0.0.1:6790`. The
   bundled Vifu binary serves the Dashboard at the same address.
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
`Python: workshop-guide`. The Agents page shows the `scene-planner` Agent and
its current prompt. Open **scene-planner → Prompt**, edit the instructions, and
select **Save & make live**. Vifu sends the live prompt to the connected Python
Runtime; the next call receives it through `request.instructions`. The trace
shows the `validate` and `provider.invoke` stages.

The `instructions=` value in Python seeds the first Agent version. Later
Dashboard edits are versioned and become the active instructions only after
you save them. This keeps the initial behavior visible in source while making
development changes explicit in Vifu.

Create another directory with another `Vifu("name")` program to create another
App. Both Apps stay available in the same personal Dashboard and keep separate
Agents, sessions, endpoints, Devices, and traces.

## Configure Local Server Startup From Python

The default `Vifu("name")` path needs no Server configuration. When an App
needs a different local address or an existing Vifu profile, configure the
managed Server in Python:

```python
from vifu import Vifu, VifuServerConfig

app = Vifu(
    "workshop-guide",
    server_config=VifuServerConfig(
        address="http://127.0.0.1:6799",
        profile="research",
    ),
)
```

`overrides` accepts scalar Vifu configuration overrides for advanced startup
cases. These settings apply only when this Python process starts the local
Server. If a Server is already ready at `address`, the SDK reuses that process
and leaves its active configuration unchanged.

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
