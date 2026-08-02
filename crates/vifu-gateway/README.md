# vifu-gateway

Provider, protocol, relay, and session implementation used by the
[`vifu`](https://crates.io/crates/vifu) application.

Rust products can use `EmbeddedRuntimeGateway` to expose the agents in a
`VifuRuntime` configured from Project Settings through Vifu Server while continuing to call
the same Runtime directly in process. Enable the `sqlite` feature when the host
needs Runtime and Gateway session state in one SQLite database.

CLI and embedded hosts persist Runtime and Gateway session metadata through the
same SQLite Runtime store. Native hosts keep the stable Machine private key and
server-specific Device Token in their platform credential store; the private
Runtime database retains resume and guest project registration state.

Install `vifu` to run the standard application. This lower-level crate is
available for custom Agent Gateway integrations that need to compose the
implementation directly.

[Repository](https://github.com/vifudotdev/vifu) |
[Documentation](https://docs.rs/vifu-gateway)
