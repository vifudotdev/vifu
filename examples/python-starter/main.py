import os
import sys

from vifu import AgentResponse, Vifu


app = Vifu(
    "python-starter",
    data_dir=os.environ.get("VIFU_EXAMPLE_DATA_DIR"),
    capture_trace_content=True,
)


@app.agent(
    "guide",
    name="Local Guide",
    metadata={"model": "python-echo"},
)
def guide(request):
    with request.trace.stage("decode", metadata={"model": "python-echo"}):
        return AgentResponse(
            output={"text": f"Local answer: {request.input['prompt']}"},
            metadata={"model": "python-echo"},
        )


if "--once" in sys.argv:
    invocation = app.invoke(
        "guide",
        {"prompt": "Where did this agent run?"},
        session_id="first-session",
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
