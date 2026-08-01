# vifu-provider-llama

Local GGUF model provider for applications embedding `vifu-runtime`.

The provider uses llama.cpp and emits incremental text through Vifu invocation
events. Apple builds enable Metal. Model files are application data and are not
included in this crate.
