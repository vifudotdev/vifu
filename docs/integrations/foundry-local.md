# Use Foundry Local With Vifu

Foundry Local owns model discovery, download, hardware selection, loading, and
native chat inference. Vifu wraps its native chat client as a Provider. This
adds stable endpoints, sessions, Gateway routing, and comparable traces.

The examples use the small `qwen2.5-0.5b` alias. Foundry Local selects an
available variant for the device. The first run can download execution
providers and model files; later inference uses the local cache.

## Python

Initialize Foundry Local, load the model, and pass its native client to the
adapter:

```python
FoundryLocalManager.initialize(Configuration(app_name="vifu-foundry-local"))
manager = FoundryLocalManager.instance
model = manager.catalog.get_model("qwen2.5-0.5b")
model.download(lambda _progress: None)
model.load()

runtime = VifuRuntime("foundry-local-python")
register_foundry_agent(runtime, model.get_chat_client(), model="qwen2.5-0.5b")
```

The complete adapter is in
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

- `first_token`: time until the first non-empty output chunk;
- `decode`: time spent consuming the remaining chunks;
- output deltas for the host UI and optional content trace;
- model and framework identity in bounded metadata.

Pair the Runtime and compare the same prompt across model aliases or hardware.
Keep the prompt, generation settings, and stopping rules fixed when comparing
performance.
