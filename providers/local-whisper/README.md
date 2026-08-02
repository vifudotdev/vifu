# Local Whisper Provider

The `local-whisper` Provider runs speech-to-text in the `vifu` process. It
is included in the default Vifu build. Source builds require CMake, a C/C++
compiler, and libclang; follow the
[installation guide](../../docs/install.md#install-native-build-dependencies).
Its model setting belongs in the private local Provider registry:

```text
~/.vifu/providers.json
```

Place the Whisper model file under `~/.vifu/models`, then add an entry like
this:

```json
{
  "providers": [
    {
      "key": "local-transcriber",
      "name": "Local Transcriber",
      "type": "local-whisper",
      "config": {
        "model": "ggml-base.en.bin",
        "language": "en"
      }
    }
  ]
}
```

`config.model` is a file name inside `~/.vifu/models`. The optional `language`
field is passed to Whisper for transcription. Vifu exposes this provider
through the Agent Gateway as one `transcription` capability; projects,
profiles, endpoints, and keys bind to that provider instead of storing local
model paths in Server configuration.

The current local Whisper route accepts WAV audio. Other audio formats should
be converted before invoking the transcription endpoint.
