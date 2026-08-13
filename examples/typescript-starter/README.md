# TypeScript Starter

This example runs a TypeScript provider inside the WebAssembly Vifu Runtime.
It registers one Agent, reports a `decode` stage, invokes its endpoint, and
prints the local trace count.

From the repository root:

```bash
bun run --cwd npm-packages/sdk build
bun examples/typescript-starter/main.ts
```

To add Dashboard monitoring, create a one-time pairing code and connect after
registering the Agent:

```typescript
const gateway = await runtime.connect(pairingCode, { name: "TypeScript Starter" });
await gateway.waitUntilConnected();
```

Omit the pairing code on later starts. Continue with the complete
[TypeScript tutorial](../../docs/get-started/typescript.md).
