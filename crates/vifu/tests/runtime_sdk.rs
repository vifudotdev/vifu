#[cfg(feature = "runtime")]
#[test]
fn public_sdk_exposes_the_embeddable_runtime() {
    use vifu::runtime::prelude::*;

    let mut runtime = HeadlessRuntime::new();
    let advance = runtime.dispatch(RuntimeCommand::new(
        "command-1",
        "application.input",
        json!({ "text": "Hello" }),
    ));

    assert_eq!(advance.snapshot.revision, 1);
}

#[cfg(feature = "gateway")]
#[test]
fn public_sdk_exposes_gateway_provider_and_extension_contracts() {
    let _: Option<vifu::gateway::config::AgentProviderDefinition> = None;
    let _: Option<vifu::gateway::runtime_extension::RuntimeExtensionDefinition> = None;
}
