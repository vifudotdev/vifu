# @vifu/sdk

Runtime bridge for sandboxed iframe and WKWebView games.

Use this SDK inside a browser game when the game needs a small, safe boundary to
talk to its host page. It starts the runtime bridge, sends events to the host,
and calls host capabilities through JSON-RPC.

It does not require a Vifu account, Vifu Cloud, a database, or a server SDK.

## Install

```bash
npm install @vifu/sdk
```

```js
import { createVifuSDK } from "@vifu/sdk";

const vifu = createVifuSDK();
await vifu.ready();
```

For plain browser builds:

```html
<script src="/vifu-sdk.js"></script>
<script>
  window.vifu.runtime.emitEvent("game.started", { level: 1 });
</script>
```

## Runtime Events

Send a game event to the host:

```js
vifu.runtime.emitEvent("game.item.collected", {
  itemId: "silver-key",
});
```

Ask the host to open a link:

```js
vifu.runtime.openExternal({
  href: "https://example.com/help",
  label: "Open help",
});
```

## Host Capabilities

The SDK keeps host capabilities generic. The host decides what capability IDs it
supports. The game calls a capability by ID and receives the host response.

```js
const result = await vifu.invoke("example.save", {
  slot: "autosave",
  payload: { room: "library" },
});
```

Build product-specific wrappers in your host app when you need them. The SDK
stays small so it can run in self-hosted runtimes and custom hosts.

## Custom Transport

Use a custom transport when your host does not use the built-in iframe or
WKWebView bridge.

```js
const sent = [];

const vifu = createVifuSDK({
  transport: {
    kind: "custom",
    post(message) {
      sent.push(message);
    },
    start(onMessage) {
      myHost.onMessage((message) => onMessage(message));
      return () => myHost.offMessage();
    },
  },
});
```

## API Surface

- `createVifuSDK(options)` creates the runtime bridge.
- `createGameRuntimeSDK(options)` is an explicit alias for runtime-focused code.
- `createClient(options)` is a compatibility alias for `createVifuSDK(options)`.
- `vifu.ready()` waits for the host bridge to connect.
- `vifu.status()` returns SDK, protocol, transport, and host connection status.
- `vifu.runtime.emitEvent(type, data, options)` sends an event to the host.
- `vifu.runtime.openExternal(input)` asks the host to open a link.
- `vifu.invoke(capabilityId, args, options)` calls a host capability.

## Package Boundary

`@vifu/sdk` is browser game runtime code. It has no cloud service dependency and
does not include product-specific APIs. Those belong in the host application or a
separate package.
