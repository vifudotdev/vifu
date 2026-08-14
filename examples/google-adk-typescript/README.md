# Google ADK With Vifu For TypeScript

This example exposes an embedded Vifu endpoint as a Google ADK `FunctionTool`.
The Vifu Runtime runs as Rust WebAssembly in Node.js. The provider is a normal
TypeScript function.

## Run It

Build the Vifu TypeScript SDK from the repository root:

```bash
bun run --cwd npm-packages/sdk build
```

In a separate application workspace, install Google ADK, its development
tools, Zod, and the packed Vifu SDK:

```bash
cd npm-packages/sdk
npm pack
cd /path/to/your-adk-app
npm install @google/adk zod /path/to/vifu/vifu-sdk-0.1.13.tgz
npm install --save-dev @google/adk-devtools
```

Copy `agent.ts` and `vifu-tool.ts` into that application, update the Vifu
import to `@vifu/sdk`, and run:

```bash
npx @google/adk-devtools run agent.ts
```

Replace the example provider with your on-device model adapter. See the
[Google ADK integration guide](../../docs/integrations/google-adk.md) for
pairing, tracing, and the boundary between ADK and Vifu.
