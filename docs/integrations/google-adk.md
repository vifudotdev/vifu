# Use Google ADK With Vifu

Google ADK can use a Vifu endpoint as a custom function tool. ADK owns the
outer agent, model selection, and tool decision. Vifu owns the delegated local
Agent, its stable endpoint, session state, Gateway connection, and trace.

## Python

Google ADK accepts an annotated Python function as a tool. The adapter calls
the embedded Vifu Runtime directly:

```python
def ask_on_device(prompt: str) -> dict:
    """Run a prompt through the on-device Vifu agent."""
    result = runtime.invoke("on-device-task", {"prompt": prompt})
    return {
        "output": result.output,
        "vifu_invocation_id": result.invocation_id,
    }

root_agent = Agent(
    name="vifu_device_router",
    model="gemini-2.5-flash",
    tools=[ask_on_device],
)
```

Run the complete [`google-adk-python`](../../examples/google-adk-python/)
example with `adk run`.

## TypeScript

Create an ADK `FunctionTool` with a Zod parameter schema. Its `execute`
function invokes the WebAssembly-backed Vifu Runtime:

```typescript
const askOnDevice = new FunctionTool({
  name: "ask_on_device",
  description: "Runs a task through the on-device Vifu agent.",
  parameters: z.object({ prompt: z.string() }),
  execute: createVifuToolHandler(runtime, "on-device-task"),
});
```

Run the complete
[`google-adk-typescript`](../../examples/google-adk-typescript/) example with
the official ADK development tools.

## Connect the Delegated Runtime

Use the high-level `Vifu` API when the ADK process owns the application
lifecycle. It connects to the local Vifu Server without a pairing code. Use the
lower-level Gateway API for a remote Server or an explicit deployment.

The Vifu trace covers work inside the Vifu endpoint. ADK's outer reasoning is
owned by ADK. Correlate the two systems by returning the Vifu invocation ID
from the tool, as both examples do.

The examples use an ADK model to choose the tool. Replace the sample Vifu
Provider with the on-device model or framework used by the product.
