# @vifu/protocol

Shared TypeScript definitions for Vifu transport-neutral protocol messages.

Vifu uses a small JSON frame model across embedded and network transports:

- `type: "req"` with `id`, `method`, and optional `params`
- `type: "res"` with `id`, `ok`, and optional `payload` or `error`
- `type: "event"` with `event` and optional `payload`

This package defines two contracts on that frame model:

- Runtime Bridge methods and events used by native hosts, Godot, Unity, Unreal,
  and other application adapters;
- Agent Gateway methods and events used to connect remote provider resources to
  Vifu Server.

Runtime Bridge frames can cross an in-process FFI boundary or a WebSocket
without changing application behavior. Product-facing Server endpoints remain
HTTP contracts and are intentionally kept outside this package.

Gateways that advertise `agent.invocation-activity.v1` receive an
`agent.invocationActivity.ready` event from a compatible server. While an
invocation is making progress they may then send throttled
`agent.invocationActivity` events. In that negotiated mode, `timeoutMs` is the
maximum idle interval rather than a hard cap on total model execution time.

## Contract Testing

The JSON files in `fixtures/gateway-frame/` and `fixtures/runtime-bridge/` are
shared protocol contracts. TypeScript and Rust tests parse these fixtures so a
frame change must keep both language implementations compatible.
