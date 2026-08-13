# Framework And Model Integrations

Vifu can sit around an existing agent framework or model runtime. The adapter
registers that system as a Vifu Provider and reports the stages that matter to
the application.

| Integration | Python example | TypeScript example | Guide |
| --- | --- | --- | --- |
| Google Agent Development Kit | [`google-adk-python`](../../examples/google-adk-python/) | [`google-adk-typescript`](../../examples/google-adk-typescript/) | [Google ADK](google-adk.md) |
| Foundry Local native chat | [`foundry-local-python`](../../examples/foundry-local-python/) | [`foundry-local-typescript`](../../examples/foundry-local-typescript/) | [Foundry Local](foundry-local.md) |

Use Vifu as the inner execution layer when a framework owns orchestration. Use
Vifu as the outer Runtime when a model SDK owns inference. In both shapes, one
stable Vifu endpoint can be invoked by application code or through a paired
Gateway.
