# Vifu TypeScript SDK

`@vifu/sdk` embeds the Rust Vifu Runtime in Node.js through WebAssembly. A
TypeScript process can register providers and agents, invoke named endpoints,
and collect the same trace records used by other Vifu hosts.

The package build requires the Rust `wasm32-unknown-unknown` target and the
matching `wasm-bindgen` CLI. See the TypeScript starter in
[`examples/typescript-starter`](../../examples/typescript-starter/README.md).

The public API provides:

- `VifuRuntime` for Agent registration, invocation, snapshots, and local traces;
- `VifuAgentTrace` for activity, output deltas, and typed provider stages;
- `VifuGateway` for pairing, stored identity, reconnect, remote invocation, and
  Dashboard telemetry through the packaged native companion;
- `VifuServer` for managing the complete installed Vifu process.

Start with the [TypeScript tutorial](../../docs/get-started/typescript.md).
Runnable adapters are available for
[Google ADK](../../examples/google-adk-typescript/) and
[Foundry Local](../../examples/foundry-local-typescript/).

This revision targets Node.js. The Runtime is WebAssembly, but the secure
Gateway companion requires native process APIs. The source build and local
package archive are the supported installation path in this revision.
