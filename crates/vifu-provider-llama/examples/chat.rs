use std::io::{self, Write as _};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vifu_provider_llama::{LlamaProvider, LlamaProviderConfig};
use vifu_runtime::{
    AgentDefinition, EndpointDefinition, InvocationData, InvocationEventKind, InvocationInput,
    InvocationStatus, VifuRuntime,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let model_path = arguments.next().ok_or(
        "usage: cargo run -p vifu-provider-llama --example chat -- <model.gguf> [message]",
    )?;
    let message = arguments
        .next()
        .unwrap_or_else(|| "Introduce yourself in one short sentence.".to_string());

    let runtime = VifuRuntime::new("local-companion")?;
    runtime.register_provider(
        "local-llama",
        Arc::new(LlamaProvider::load(LlamaProviderConfig::new(model_path))?),
    )?;
    runtime.register_agent(AgentDefinition {
        id: "nova".to_string(),
        name: "Nova".to_string(),
        provider: "local-llama".to_string(),
        capabilities: vec!["chat".to_string()],
        metadata: json!({
            "instructions": "You are Nova, a warm companion. Reply briefly and directly."
        }),
    })?;
    runtime.register_endpoint(EndpointDefinition {
        name: "companion".to_string(),
        agent: "nova".to_string(),
        capability: "chat".to_string(),
        timeout_ms: 120_000,
    })?;

    let handle = runtime.start_invoke(InvocationInput::json(
        "companion",
        json!({
            "messages": [{ "role": "user", "content": message }],
            "maxTokens": 96,
            "temperature": 0.7,
            "topP": 0.9,
        }),
    ))?;
    let started = Instant::now();
    let mut first_token = None;
    let mut output = String::new();

    loop {
        for event in runtime.drain_invocation_events(&handle)? {
            if event.kind != InvocationEventKind::OutputDelta {
                continue;
            }
            let Some(InvocationData::Json(Value::String(delta))) = event.data else {
                continue;
            };
            first_token.get_or_insert_with(|| started.elapsed());
            output.push_str(&delta);
            print!("{delta}");
            io::stdout().flush()?;
        }

        let poll = runtime.poll_invocation(&handle)?;
        match poll.status {
            InvocationStatus::Pending | InvocationStatus::Running => {
                std::thread::sleep(Duration::from_millis(15));
            }
            InvocationStatus::Completed => {
                let completed = runtime.take_invocation(&handle)?;
                let token_count = completed
                    .output
                    .as_ref()
                    .and_then(|value| value.metadata.get("outputTokens"))
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
                let elapsed = started.elapsed();
                println!(
                    "\n\nTTFT: {:.2}s | Output: {} tokens | {:.1} tokens/s",
                    first_token.unwrap_or(elapsed).as_secs_f64(),
                    token_count,
                    token_count as f64 / elapsed.as_secs_f64().max(0.001),
                );
                break;
            }
            InvocationStatus::Failed => {
                return Err(poll
                    .error
                    .unwrap_or_else(|| "local inference failed".to_string())
                    .into());
            }
            InvocationStatus::Cancelled => return Err("local inference was cancelled".into()),
        }
    }

    Ok(())
}
