"""Google ADK agent that delegates a task to an embedded Vifu agent."""

from google.adk import Agent

from vifu import AgentResponse, VifuRuntime

from .vifu_tool import create_vifu_tool


runtime = VifuRuntime("google-adk-python")


def local_provider(request):
    prompt = request.input["prompt"]
    with request.trace.stage("decode", metadata={"model": "example-local-provider"}):
        return AgentResponse(
            output={"text": f"On-device result: {prompt}"},
            metadata={"model": "example-local-provider"},
        )


runtime.agent(
    "local-reasoner",
    local_provider,
    endpoint="on-device-task",
    metadata={"model": "example-local-provider"},
)

root_agent = Agent(
    name="vifu_device_router",
    model="gemini-2.5-flash",
    instruction=(
        "Use ask_on_device when the user asks you to run a task on the local device. "
        "Return the tool result clearly."
    ),
    tools=[create_vifu_tool(runtime, "on-device-task")],
)
