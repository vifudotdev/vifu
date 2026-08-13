import os

from vifu import AgentResponse, VifuRuntime


runtime = VifuRuntime(
    "python-starter",
    data_dir=os.environ.get("VIFU_EXAMPLE_DATA_DIR"),
)


def local_guide(request):
    with request.trace.stage("decode", metadata={"model": "python-echo"}):
        return AgentResponse(
            output={"text": f"Local answer: {request.input['prompt']}"},
            metadata={"model": "python-echo"},
        )


runtime.agent(
    "guide",
    local_guide,
    name="Local Guide",
    metadata={"model": "python-echo"},
)

invocation = runtime.invoke(
    "guide",
    {"prompt": "Where did this agent run?"},
    session_id="first-session",
)

print(invocation.output)
print(
    {
        "invocation_id": invocation.invocation_id,
        "duration_ms": invocation.trace[0]["durationMs"],
        "pending_traces": len(runtime.pending_traces()),
    }
)
