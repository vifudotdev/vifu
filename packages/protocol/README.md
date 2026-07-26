# @vifu/protocol

Shared TypeScript definitions for Vifu gateway protocol messages.

Vifu uses a small JSON frame model for gateway transports:

- `type: "req"` with `id`, `method`, and optional `params`
- `type: "res"` with `id`, `ok`, and optional `payload` or `error`
- `type: "event"` with `event` and optional `payload`

This package also defines the current Vifu Agent Gateway method, event, and
payload shapes. Product APIs such as endpoint invocation are HTTP contracts and
are intentionally kept outside this gateway frame package.

## Contract Testing

The JSON files in `fixtures/gateway-frame/` are the shared protocol contract for
TypeScript and Rust. Both `@vifu/protocol` tests and `vifu-gateway` tests parse the
same fixture directory, so adding or changing a gateway frame fixture must keep
both language implementations compatible.
