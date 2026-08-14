# Use Foundry Local With Vifu

Foundry Local owns model discovery, download, hardware selection, loading, and
native chat inference. Vifu wraps its native chat client as a Provider. This
adds stable endpoints, sessions, Gateway routing, and comparable traces.

The examples use the small `qwen2.5-0.5b` alias. Foundry Local selects an
available variant for the device. The first run can download execution
providers and model files. Later inference uses the local cache.

## Python

Install the Python packages:

```bash
python -m pip install --upgrade "vifu[foundry]"
```

On Windows, install `"vifu[foundry-winml]"` for Windows ML acceleration. Install
one Foundry Local package variant in an environment.

Create the Vifu application:

```python
from vifu import Vifu
from vifu.integrations.foundry import FoundryLocal

app = Vifu("Foundry Local Chat", capture_trace_content=True)
app.agent("chat", FoundryLocal("qwen2.5-0.5b"))
app.run()
```

The integration prepares the execution provider and loads the model. It
downloads the model when necessary. Vifu starts the local Server and Agent
Gateway. The application unloads its model and stops its owned processes when
it exits.

The complete application is in
[`foundry-local-python`](../../examples/foundry-local-python/).

## TypeScript

```typescript
const manager = FoundryLocalManager.create({ appName: "vifu-foundry-local" });
const model = await manager.catalog.getModel("qwen2.5-0.5b");
await model.download(() => {});
await model.load();

const runtime = new VifuRuntime("foundry-local-typescript");
registerFoundryAgent(runtime, model.createChatClient(), {
  model: "qwen2.5-0.5b",
});
```

The complete adapter is in
[`foundry-local-typescript`](../../examples/foundry-local-typescript/).

## What The Adapter Reports

Both adapters consume Foundry Local's streaming API. They report:

- `first_token`: time until the first non-empty output chunk.
- `decode`: time spent consuming the remaining chunks.
- Output deltas for the host UI and optional content trace.
- model and framework identity in bounded metadata.

The Python example serves terminal prompts and remote endpoint calls through
the same Agent.

Use the same prompt to compare model aliases or hardware. Keep the generation
settings and stopping rules fixed during each comparison.
