# vifu-provider-llama

Local GGUF model provider for applications embedding `vifu-runtime`.

The provider uses llama.cpp and emits incremental text through Vifu invocation
events. Apple builds enable Metal. Model files are application data and are not
included in this crate.

The `vifu` binary reads model paths and resource limits from the local
`providers.json`; see the [Local llama Provider guide](https://github.com/vifudotdev/vifu/tree/main/providers/llama).
Embedded Rust and Apple hosts can also construct `LlamaProviderConfig` directly.

Pass `mmprojPath` beside a matching vision-language GGUF to enable in-process
image understanding through standard OpenAI `image_url` chat content. Embedded
Rust hosts can use `LlamaProvider::load_multimodal` with
`LlamaMultimodalConfig` for the same capability.
