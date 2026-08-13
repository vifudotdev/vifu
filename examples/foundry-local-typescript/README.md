# Foundry Local With Vifu For TypeScript

This example registers a Foundry Local native chat client as a Vifu provider.
The Vifu Runtime runs as Rust WebAssembly in Node.js. Foundry Local performs
model inference on the device.

## Run It

Build and pack the Vifu TypeScript SDK from the repository root:

```bash
bun run --cwd npm-packages/sdk build
cd npm-packages/sdk && npm pack
```

In a separate application workspace, install Foundry Local and the packed Vifu
SDK:

```bash
npm install foundry-local-sdk /path/to/vifu/vifu-sdk-0.1.12.tgz
```

Use `foundry-local-sdk-winml` instead on supported Windows systems. Copy
`main.ts` and `provider.ts` into the application, update the Vifu import to
`@vifu/sdk`, and run the application with its TypeScript runner.

The first run can download execution providers and the selected model. The
adapter streams output and reports `first_token` and `decode` stages to Vifu.
See the [Foundry Local integration guide](../../docs/integrations/foundry-local.md)
for pairing and Dashboard inspection.
