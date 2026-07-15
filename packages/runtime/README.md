# @vifu/runtime

Browser runtime host utilities for iframe-based games.

Use this package when a web app needs to load a game in an iframe and exchange
messages with that runtime.

It provides iframe attributes, runtime URL helpers, viewport sync messages,
JSON-RPC host messaging, and event batching primitives.

## Install

```bash
npm install @vifu/runtime
```

## Iframe Setup

```ts
import {
  RUNTIME_IFRAME_ALLOW,
  RUNTIME_IFRAME_SANDBOX,
  RUNTIME_IFRAME_SCROLLING,
  withRuntimeIframeParams,
} from "@vifu/runtime";

const src = withRuntimeIframeParams(runtimeUrl, gameId);
```

```tsx
<iframe
  src={src}
  allow={RUNTIME_IFRAME_ALLOW}
  sandbox={RUNTIME_IFRAME_SANDBOX}
  scrolling={RUNTIME_IFRAME_SCROLLING}
/>
```

## Sandbox Policy

Use `runtimeIframeSandboxForTemplate` when your app needs template-specific
sandbox settings.

```ts
import { runtimeIframeSandboxForTemplate } from "@vifu/runtime";

const sandbox = runtimeIframeSandboxForTemplate(templateType, templateId, {
  sameOriginTemplateIds: ["your.first-party-template"],
});
```

## Runtime Host

Use `createRuntimeHost` to send JSON-RPC messages to an iframe runtime and
receive messages from it.

```ts
import { createRuntimeHost } from "@vifu/runtime";

const host = createRuntimeHost({
  iframe,
  onMessage(message) {
    console.log(message.method);
  },
});

host.post("host.ready", { ok: true });
```

## Viewport Sync

Use `observeRuntimeIframeViewport` when the runtime needs resize messages from
the host page.

```ts
import { observeRuntimeIframeViewport } from "@vifu/runtime";

const stop = observeRuntimeIframeViewport(iframe);
```

## Event Bus

Use `getCloudEventBus` to collect events posted by registered game iframes and
send them through your own transport.

```ts
import { getCloudEventBus } from "@vifu/runtime";

const bus = getCloudEventBus({
  async sendEvents(events) {
    await fetch("/events", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(events),
    });
  },
});

const unregister = bus.registerGameIframe(iframe, `/games/${gameId}`);
```
