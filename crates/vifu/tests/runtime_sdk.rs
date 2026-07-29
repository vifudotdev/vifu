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

#[cfg(feature = "runtime")]
#[test]
fn public_sdk_invokes_a_registered_provider_without_server_or_gateway() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use vifu::runtime::prelude::*;

    struct EchoProvider;

    impl AgentProvider for EchoProvider {
        fn supports(&self, capability: &str) -> bool {
            capability == "chat"
        }

        fn invoke<'a>(
            &'a self,
            request: ProviderRequest,
            _cancellation: CancellationToken,
        ) -> ProviderFuture<'a> {
            Box::pin(async move {
                Ok(ProviderResponse {
                    data: request.data,
                    metadata: json!({}),
                    state: None,
                })
            })
        }
    }

    let runtime = VifuRuntime::new("embedded-app").unwrap();
    runtime
        .register_provider("echo", Arc::new(EchoProvider))
        .unwrap();
    runtime
        .register_agent(AgentDefinition {
            id: "guide".to_string(),
            name: "Guide".to_string(),
            provider: "echo".to_string(),
            capabilities: vec!["chat".to_string()],
            metadata: json!({}),
        })
        .unwrap();
    runtime
        .register_endpoint(EndpointDefinition {
            name: "guide".to_string(),
            agent: "guide".to_string(),
            capability: "chat".to_string(),
            timeout_ms: 500,
        })
        .unwrap();
    let handle = runtime
        .start_invoke(InvocationInput::json("guide", json!({ "text": "Hello" })))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    let output = loop {
        let poll = runtime.poll_invocation(&handle).unwrap();
        if let Some(output) = poll.output {
            break output;
        }
        assert!(Instant::now() < deadline, "embedded invocation timed out");
        std::thread::sleep(Duration::from_millis(5));
    };

    assert_eq!(
        output.data,
        InvocationData::Json(json!({ "text": "Hello" }))
    );
}

#[cfg(feature = "gateway")]
#[test]
fn public_sdk_exposes_gateway_provider_and_extension_contracts() {
    let _: Option<vifu::gateway::config::AgentProviderDefinition> = None;
    let _: Option<vifu::gateway::runtime_extension::RuntimeExtensionDefinition> = None;
}
