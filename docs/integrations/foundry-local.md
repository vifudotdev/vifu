# Add Vifu to a Foundry Local App

Foundry Local keeps ownership of model discovery, download, hardware selection,
loading, inference, and unloading. Vifu adds the application boundary, Agent
identity, sessions, endpoints, Device connection, and inference-stage traces.

## 1. Keep the Foundry Local Setup

Install the packages:

```bash
python -m pip install --upgrade "vifu[foundry]"
```

On Windows, use `"vifu[foundry-winml]"` for Windows ML acceleration.

Start with the normal Foundry Local lifecycle:

```python
from foundry_local_sdk import Configuration, FoundryLocalManager

FoundryLocalManager.initialize(Configuration(app_name="my-app"))
model = FoundryLocalManager.instance.catalog.get_model("qwen2.5-0.5b")
model.download()
model.load()
client = model.get_chat_client()
```

Vifu does not replace these objects or make lifecycle decisions for them.

## 2. Add the Vifu App and Agent

Register the application behavior that already uses `client`:

```python
from vifu import Vifu
from vifu.integrations.foundry import foundry_chunk_text, trace_foundry_stream

app = Vifu("web-research", capture_trace_content=True)


@app.agent(
    "researcher",
    capability="research",
    metadata={"framework": "foundry-local", "model": "qwen2.5-0.5b"},
)
def research(request):
    messages = [{"role": "user", "content": request.input["research_prompt"]}]
    chunks = client.complete_streaming_chat(messages)
    observed = trace_foundry_stream(
        request,
        chunks,
        model="qwen2.5-0.5b",
    )
    answer = "".join(foundry_chunk_text(chunk) for chunk in observed)
    return {"answer": answer}
```

The Foundry method call and its native chunks remain visible in application
code. `trace_foundry_stream` yields those same chunks. It records
`first_token`, `decode`, and output-delta telemetry while they pass through.
`foundry_chunk_text` returns empty text for Foundry control chunks, so output
assembly does not assume every chunk contains a model choice.

## 3. Run Existing Product Logic

```python
def run_my_app(vifu):
    sources = search_web("Arm-optimized on-device AI")
    result = vifu.invoke(
        "researcher",
        {"research_prompt": prompt_with_citations(sources)},
        session_id="arm-research",
    )
    publish_brief(result.output["answer"], sources)


try:
    app.run(run_my_app)
finally:
    model.unload()
```

`search_web`, `prompt_with_citations`, `run_my_app`, and `publish_brief` belong
to the developer's application. Vifu does not turn the program into a chat
shell. On first run, it creates the App
in the personal Server and stores the stable binding in `.vifu/app.json`.

The complete runnable form is in
[`foundry-local-python`](../../examples/foundry-local-python/).

## Compare Arm Behavior

Keep the application input and generation settings fixed. Change the Foundry
model variant or execution provider, then compare `first_token`, `decode`, and
total latency in the App's Traces page. These measurements show the observed
runtime behavior; Vifu does not claim to optimize the model itself.
