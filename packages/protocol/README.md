# @vifu/protocol

Shared TypeScript definitions for Vifu protocol messages.

This package currently includes gateway frame types:

- `type: "req"` with `id`, `method`, and optional `params`
- `type: "res"` with `id`, `ok`, and optional `payload` or `error`
- `type: "event"` with `event` and optional `payload`

It also includes helper types for the current Project JSON-RPC endpoint and
discovery payload.
