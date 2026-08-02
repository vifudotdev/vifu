# Local llama Provider

The `llama` Provider loads one GGUF model into the `vifu` process through
llama.cpp. It is included in the default Vifu build. Source builds require
CMake, a C/C++ compiler, and libclang; follow the
[installation guide](../../docs/install.md#install-native-build-dependencies).
Its model and resource settings belong in the private local Provider registry:

```text
~/.vifu/providers.json
```

Add an entry like this:

```json
{
  "providers": [
    {
      "key": "local-qwen",
      "name": "Local Qwen",
      "type": "llama",
      "config": {
        "modelPath": "models/Qwen3-1.7B-Q4_K_M.gguf",
        "contextSize": 4096,
        "defaultMaxTokens": 256,
        "maxConcurrency": 1
      }
    }
  ]
}
```

Relative `modelPath` values resolve from the directory containing
`providers.json`. Omit `gpuLayers` to let llama.cpp offload all supported
layers; set it to `0` for CPU-only execution.

Start Vifu normally:

```bash
vifu
```

The Gateway loads the model once and advertises one chat-capable resource using
the Provider `key`. Assign that Provider to a project and create profiles or
Runtime manifests that bind Agent behavior to it. Profile persona instructions
are applied per invocation, so multiple profiles can share the resident model.

The supported `config` fields are:

| Field | Default | Meaning |
| --- | --- | --- |
| `modelPath` | required | Local GGUF path |
| `contextSize` | `4096` | Maximum context tokens |
| `gpuLayers` | all supported layers | Layers offered to the available accelerator backend |
| `defaultMaxTokens` | `256` | Generation limit when a request omits one |
| `maxConcurrency` | `1` | Simultaneous generations accepted by this model instance |
| `mmprojPath` | none | Matching llama.cpp multimodal projector; enables image input for `chat` |
| `mmprojUseGpu` | follows `gpuLayers` | Offer the multimodal projector to the accelerator backend |
| `imageMinTokens` | model default | Minimum visual token budget, or `-1` for the model default |
| `imageMaxTokens` | model default | Maximum visual token budget, or `-1` for the model default |
| `maxImages` | `8` | Maximum images accepted in one chat request |
| `maxMediaBytes` | `16777216` | Maximum decoded image bytes accepted in one chat request |

Vifu rejects unknown fields, zero-sized contexts, generation limits outside
`1..=2048`, and concurrency outside `1..=64` before loading the model.

## On-device vision

Use a vision-language GGUF together with its matching projector:

```json
{
  "providers": [
    {
      "key": "local-vision",
      "name": "Local Vision",
      "type": "llama",
      "config": {
        "modelPath": "models/Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf",
        "mmprojPath": "models/mmproj-Qwen2.5-VL-3B-Instruct-f16.gguf",
        "contextSize": 8192,
        "imageMaxTokens": 512,
        "maxConcurrency": 1
      }
    }
  ]
}
```

The normal OpenAI-compatible `chat/completions` endpoint then accepts ordered
`text` and `image_url` content parts. The in-process Provider accepts bounded
`data:image/...;base64,...` URLs and does not fetch remote image URLs. Vifu
records image size and digest metadata in traces while omitting the base64
payload.
