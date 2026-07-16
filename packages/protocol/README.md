# @vifu/protocol

Shared TypeScript definitions for Vifu gateway protocol messages.

Vifu uses a small JSON frame model for gateway transports:

- `type: "req"` with `id`, `method`, and optional `params`
- `type: "res"` with `id`, `ok`, and optional `payload` or `error`
- `type: "event"` with `event` and optional `payload`

This package also defines the current Vifu Agent Gateway method, event, and
payload shapes. Product APIs such as endpoint invocation are HTTP contracts and
are intentionally kept outside this gateway frame package.
