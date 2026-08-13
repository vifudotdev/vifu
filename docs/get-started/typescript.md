# Build A TypeScript Agent With Vifu

This tutorial embeds Vifu in Node.js. The Agent Runtime is Rust compiled to
WebAssembly. Provider code is TypeScript. A packaged native companion handles
Gateway identity, secure reconnect, remote invocation, and trace upload.

The package is currently built from this repository.

## 1. Build The SDK

From the Vifu repository root:

```bash
bun run --cwd npm-packages/sdk build
```

The build produces `npm-packages/sdk/dist`. To use it in another application,
create and inspect a local package archive:

```bash
cd npm-packages/sdk
npm pack
```

Install that archive in the application workspace. The archive contains the
WebAssembly Runtime and the native Gateway companion for the build platform.

## 2. Register An Agent

```typescript
import { VifuRuntime } from "@vifu/sdk";

const runtime = new VifuRuntime("my-typescript-app");

runtime.agent({
  id: "guide",
  endpoint: "chat",
  metadata: { model: "my-local-model" },
  async handler(request, trace) {
    const input = request.input as { prompt: string };
    return trace.stage(
      "decode",
      async () => ({
        output: { text: `Local answer: ${input.prompt}` },
        metadata: { model: "my-local-model" },
      }),
      { model: "my-local-model" },
    );
  },
});
```

## 3. Invoke The Endpoint

```typescript
const result = await runtime.invoke({
  endpoint: "chat",
  sessionId: "player-42",
  input: { prompt: "Explain the next step." },
});

console.log(result.output);
console.log(result.invocationId);
console.log(result.trace);
```

Use `trace.activity()` during long work and `trace.outputDelta(...)` for
streaming output. Use the typed stages `queue`, `load`, `tokenize`, `prefill`,
`first_token`, `decode`, and `validate` for comparable measurements.

## 4. Pair With The Dashboard

Create a one-time pairing code in the App's Devices page:

```typescript
const gateway = await runtime.connect(pairingCode, {
  name: "TypeScript model on my laptop",
});
await gateway.waitUntilConnected();
```

On later starts, call `runtime.connect(undefined, { name: "..." })`. The SDK
uses its stored device identity and Server token. Call `gateway.close()` and
`runtime.close()` during application shutdown.

Content capture is off by default. Set `captureTraceContent: true` only after
the host application has obtained user consent.

## 5. Let TypeScript Manage A Local Server

```typescript
import { VifuServer } from "@vifu/sdk";

const server = await VifuServer.start();
console.log(server.running);
await server.close();
```

This starts the complete installed `vifu` binary. The current TypeScript SDK is
for Node.js. Its Runtime is WebAssembly, but the secure Gateway companion uses
native process APIs and is not a browser API.

## 6. Continue From A Working Example

Run [`examples/typescript-starter`](../../examples/typescript-starter/) first.
Then use the [Google ADK](../integrations/google-adk.md) or
[Foundry Local](../integrations/foundry-local.md) guide.
