import os
import sys
from pathlib import Path

from vifu import AgentResponse, Vifu


app = Vifu(
    "python-starter",
    data_dir=os.environ.get("VIFU_EXAMPLE_DATA_DIR"),
    workspace=Path(__file__).parent,
    capture_trace_content=True,
)


@app.agent(
    "scene-planner",
    name="Scene Planner",
    capability="planning",
    metadata={"provider": "python-rules"},
)
def scene_planner(request):
    with request.trace.stage("plan", metadata={"provider": "python-rules"}):
        return AgentResponse(
            output={
                "action": "inspect",
                "target": request.input["scene"],
            },
            metadata={"provider": "python-rules"},
        )


if "--once" in sys.argv:
    with app:
        invocation = app.invoke(
            "scene-planner",
            {"scene": "workshop-door"},
            session_id="demo-run",
        )
    print(invocation.output)
    print(
        {
            "invocation_id": invocation.invocation_id,
            "duration_ms": invocation.trace[0]["durationMs"],
            "pending_traces": len(app.runtime.pending_traces()),
        }
    )
else:
    app.run()
