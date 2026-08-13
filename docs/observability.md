# Trust Vifu Trace Data

Vifu makes each measurement attributable. A trace identifies the App,
deployment, Gateway/device, Agent, Provider, model metadata, endpoint, status,
and invocation. This lets a developer answer three questions: where did the
request run, what ran, and how long did each stage take?

## Report The Work Your Provider Performs

Use these typed stages consistently:

| Stage | Start | End | Useful metadata |
| --- | --- | --- | --- |
| `queue` | Work enters a local queue | Execution starts | queue depth |
| `load` | Model loading starts | Model is ready | model, backend, resident |
| `tokenize` | Input conversion starts | Input tokens are ready | input tokens |
| `prefill` | Prompt evaluation starts | Decode can start | input tokens, context size |
| `first_token` | Generation is requested | First non-empty token arrives | model, backend |
| `decode` | First token is available | Generation stops | output tokens |
| `validate` | Output checks start | Output is accepted or rejected | rule or schema name |

Vifu records the measured duration supplied by the Provider. It does not infer
GPU use, token count, or optimization status from a model name. Report those
values only when the Provider can measure them.

## Read A Trace

The Dashboard trace list shows status, Agent, Gateway, latency, time to first
token, tokens per second, and available score data. Trace detail shows the
Gateway/device, model metadata, token counts, stage timeline, safe errors, and
optional input/output content.

Use the filters to isolate one Agent, Gateway, status, date range, or search
term. Use readable Gateway names and model metadata. Do not use a Gateway ID as
the only device label.

## Compare Two Runs

For an optimization comparison:

1. Use the same device, model file, prompt, context size, output limit, and
   stopping rules.
2. Run a warm-up request for each build.
3. Record several measured requests, not one screenshot.
4. Filter traces by Gateway or Provider build.
5. Compare `load`, `prefill`, `first_token`, `decode`, total latency, and
   errors separately.

The Android optimized and baseline Starters can be installed together. Pair
both with one App, give each Gateway a readable name, and compare their traces.
The same method works for Python, TypeScript, Swift, Kotlin, Godot, Rust, and
other embedded devices.

## Content And Privacy

Gateway monitoring is content-private by default. Vifu sends lifecycle,
identity, performance data, model metadata, token counts, status, and bounded
errors. Root prompt and output content remain on the device.

An application can enable bounded content capture after an explicit user
consent decision. Treat uploaded content as application data: remove secrets,
personal data, and unsupported fields before capture. The host application
owns that policy and consent UI.
