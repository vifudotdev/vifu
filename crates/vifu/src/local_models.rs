use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{oneshot, watch, Mutex};
use vifu_gateway::relay::{
    AgentGatewayProvider, CancellationToken, GatewayProviderError, InProcessGatewayProvider,
    ProviderEventSink,
};
use vifu_provider_llama::LlamaProvider;

const DEFAULT_MODEL_MEMORY_FRACTION: u64 = 60;
const FALLBACK_MODEL_MEMORY_BUDGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Shared, lazy pool for configured llama.cpp models.
///
/// A pool entry exists as cheap configuration until its first invocation. Two
/// provider keys with the same effective configuration reuse one loaded model.
#[derive(Clone)]
pub struct LocalModelPool {
    state: Arc<Mutex<PoolState>>,
    residency: Arc<RwLock<ResidencyState>>,
    load_lock: Arc<Mutex<()>>,
    memory_budget_bytes: u64,
    loaded_count: Arc<AtomicUsize>,
    access_sequence: Arc<AtomicU64>,
    #[cfg(test)]
    test_loader: Option<TestModelLoader>,
}

#[cfg(test)]
type TestModelLoader =
    Arc<dyn Fn(Value, PathBuf) -> Result<Arc<LlamaProvider>, String> + Send + Sync>;

#[derive(Default)]
struct PoolState {
    slots: HashMap<String, Arc<ModelSlot>>,
}

#[derive(Default)]
struct ResidencyState {
    provider_models: HashMap<String, String>,
    active_routes: HashMap<String, String>,
}

struct ModelSlot {
    config: Value,
    base_dir: PathBuf,
    estimated_bytes: u64,
    last_used_sequence: AtomicU64,
    provider: OnceLock<Arc<LlamaProvider>>,
    load_state: Mutex<ModelLoadState>,
}

enum ModelLoadState {
    Idle,
    Loading(watch::Receiver<Option<Result<Arc<LlamaProvider>, String>>>),
}

impl LocalModelPool {
    pub fn for_device() -> Self {
        let memory_budget_bytes = physical_memory_bytes()
            .map(|total| total.saturating_mul(DEFAULT_MODEL_MEMORY_FRACTION) / 100)
            // Non-Unix targets without a physical-memory probe still enforce a
            // conservative hard cap instead of silently admitting every model.
            .unwrap_or(FALLBACK_MODEL_MEMORY_BUDGET_BYTES);
        Self::with_memory_budget(memory_budget_bytes)
    }

    fn with_memory_budget(memory_budget_bytes: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(PoolState::default())),
            residency: Arc::new(RwLock::new(ResidencyState::default())),
            load_lock: Arc::new(Mutex::new(())),
            memory_budget_bytes,
            loaded_count: Arc::new(AtomicUsize::new(0)),
            access_sequence: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            test_loader: None,
        }
    }

    #[cfg(test)]
    fn with_test_loader(memory_budget_bytes: u64, test_loader: TestModelLoader) -> Self {
        Self {
            test_loader: Some(test_loader),
            ..Self::with_memory_budget(memory_budget_bytes)
        }
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded_count.load(Ordering::Relaxed)
    }

    fn register_provider_model(&self, provider_key: &str, cache_key: &str) {
        self.residency
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .provider_models
            .insert(provider_key.to_string(), cache_key.to_string());
    }

    /// Records the provider selected at a real invocation boundary. Models
    /// needed by another current route are protected from budget-driven LRU
    /// eviction even while no request is actively holding their provider Arc.
    pub(crate) fn set_active_route(&self, route_key: &str, provider_key: &str) {
        self.residency
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_routes
            .insert(route_key.to_string(), provider_key.to_string());
    }

    pub(crate) fn clear_active_routes(&self) {
        self.residency
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_routes
            .clear();
    }

    fn protected_cache_keys(&self) -> HashSet<String> {
        let residency = self
            .residency
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        residency
            .active_routes
            .values()
            .filter_map(|provider_key| residency.provider_models.get(provider_key))
            .cloned()
            .collect()
    }

    /// Releases every model that is not currently held by an invocation.
    /// Optimization uses this before a cold run; active calls retain their Arc.
    pub async fn evict_all_idle(&self) -> usize {
        let pool = self.clone();
        let (result_sender, result_receiver) = oneshot::channel();
        std::mem::drop(tokio::spawn(async move {
            let result = pool.evict_all_idle_owned().await;
            let _ = result_sender.send(result);
        }));
        result_receiver.await.unwrap_or_default()
    }

    async fn evict_all_idle_owned(&self) -> usize {
        let _load_guard = self.load_lock.lock().await;
        let evicted = {
            let mut state = self.state.lock().await;
            let keys = state
                .slots
                .iter()
                .filter(|(_, slot)| {
                    Arc::strong_count(slot) == 1
                        && slot
                            .provider
                            .get()
                            .is_some_and(|provider| Arc::strong_count(provider) == 1)
                })
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| state.slots.remove(&key))
                .collect::<Vec<_>>()
        };
        let count = evicted.len();
        if count > 0 {
            self.loaded_count.fetch_sub(count, Ordering::Relaxed);
            wait_for_model_drop(evicted).await;
        }
        count
    }

    async fn get_or_load(
        &self,
        config: &Value,
        base_dir: &Path,
    ) -> Result<(Arc<LlamaProvider>, bool), String> {
        let cache_key = model_cache_key(config, base_dir)?;
        if let Some(provider) = self.resident_provider(&cache_key).await {
            return Ok((provider, true));
        }
        let slot = self
            .model_slot(&cache_key, config.clone(), base_dir.to_path_buf())
            .await;
        if let Some(provider) = slot.provider.get() {
            slot.last_used_sequence
                .store(self.next_access_sequence(), Ordering::Relaxed);
            return Ok((Arc::clone(provider), true));
        }
        let mut result_receiver = {
            let mut load_state = slot.load_state.lock().await;
            if let Some(provider) = slot.provider.get() {
                slot.last_used_sequence
                    .store(self.next_access_sequence(), Ordering::Relaxed);
                return Ok((Arc::clone(provider), true));
            }
            match &*load_state {
                ModelLoadState::Loading(receiver) => receiver.clone(),
                ModelLoadState::Idle => {
                    let (result_sender, result_receiver) = watch::channel(None);
                    *load_state = ModelLoadState::Loading(result_receiver.clone());
                    let pool = self.clone();
                    let slot = Arc::clone(&slot);
                    std::mem::drop(tokio::spawn(async move {
                        pool.load_slot_owned(cache_key, slot, result_sender).await;
                    }));
                    result_receiver
                }
            }
        };
        loop {
            if let Some(result) = result_receiver.borrow().clone() {
                return result.map(|provider| (provider, false));
            }
            result_receiver.changed().await.map_err(|_| {
                "local model pool task stopped before loading completed".to_string()
            })?;
        }
    }

    async fn model_slot(
        &self,
        cache_key: &str,
        config: Value,
        base_dir: PathBuf,
    ) -> Arc<ModelSlot> {
        let mut state = self.state.lock().await;
        Arc::clone(state.slots.entry(cache_key.to_string()).or_insert_with(|| {
            Arc::new(ModelSlot {
                estimated_bytes: configured_model_bytes(&config, &base_dir),
                config,
                base_dir,
                last_used_sequence: AtomicU64::new(0),
                provider: OnceLock::new(),
                load_state: Mutex::new(ModelLoadState::Idle),
            })
        }))
    }

    async fn resident_provider(&self, cache_key: &str) -> Option<Arc<LlamaProvider>> {
        let state = self.state.lock().await;
        let slot = state.slots.get(cache_key)?;
        let provider = Arc::clone(slot.provider.get()?);
        // Clone while holding the state lock so an eviction either happens
        // before this lookup or observes the caller's pin afterwards.
        slot.last_used_sequence
            .store(self.next_access_sequence(), Ordering::Relaxed);
        Some(provider)
    }

    async fn load_slot_owned(
        &self,
        cache_key: String,
        slot: Arc<ModelSlot>,
        result_sender: watch::Sender<Option<Result<Arc<LlamaProvider>, String>>>,
    ) {
        // The pool task owns this guard independently from every waiter. It
        // publishes a provider Arc before releasing the guard, so waiters pin
        // the model before another admission can consider it idle.
        let _load_guard = self.load_lock.lock().await;
        let result = if let Some(provider) = slot.provider.get() {
            Ok(Arc::clone(provider))
        } else if let Err(error) = self.evict_for_owned(&cache_key, slot.estimated_bytes).await {
            Err(error)
        } else {
            let config = slot.config.clone();
            let base_dir = slot.base_dir.clone();
            #[cfg(test)]
            let test_loader = self.test_loader.clone();
            let loaded = tokio::task::spawn_blocking(move || {
                #[cfg(test)]
                if let Some(loader) = test_loader {
                    return loader(config, base_dir);
                }
                LlamaProvider::load_from_provider_config(&config, &base_dir)
                    .map(Arc::new)
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| format!("llama model loader task failed: {error}"))
            .and_then(|result| result);
            match loaded {
                Ok(provider) => {
                    if slot.provider.set(provider).is_ok() {
                        self.loaded_count.fetch_add(1, Ordering::Relaxed);
                    }
                    slot.provider
                        .get()
                        .cloned()
                        .ok_or_else(|| "local model loader completed without a model".to_string())
                }
                Err(error) => Err(error),
            }
        };
        if result.is_ok() {
            slot.last_used_sequence
                .store(self.next_access_sequence(), Ordering::Relaxed);
        } else {
            let mut state = self.state.lock().await;
            if state
                .slots
                .get(&cache_key)
                .is_some_and(|current| Arc::ptr_eq(current, &slot))
            {
                state.slots.remove(&cache_key);
            }
        }
        *slot.load_state.lock().await = ModelLoadState::Idle;
        let _ = result_sender.send(Some(result));
    }

    async fn evict_for_owned(
        &self,
        requested_key: &str,
        requested_bytes: u64,
    ) -> Result<(), String> {
        if requested_bytes > self.memory_budget_bytes {
            return Err(format!(
                "model admission estimate {} bytes exceeds the local model budget {} bytes",
                requested_bytes, self.memory_budget_bytes
            ));
        }
        let protected_cache_keys = self.protected_cache_keys();
        let (evicted, remaining_bytes) = {
            let mut state = self.state.lock().await;
            let mut loaded_bytes = state
                .slots
                .values()
                .filter(|slot| slot.provider.get().is_some())
                .map(|slot| slot.estimated_bytes)
                .sum::<u64>();
            if loaded_bytes.saturating_add(requested_bytes) <= self.memory_budget_bytes {
                return Ok(());
            }

            let mut candidates = state
                .slots
                .iter()
                .filter_map(|(key, slot)| {
                    let provider = slot.provider.get()?;
                    (key != requested_key
                        && !protected_cache_keys.contains(key)
                        && Arc::strong_count(slot) == 1
                        && Arc::strong_count(provider) == 1)
                        .then(|| {
                            (
                                key.clone(),
                                slot.last_used_sequence.load(Ordering::Relaxed),
                                slot.estimated_bytes,
                            )
                        })
                })
                .collect::<Vec<_>>();
            candidates.sort_unstable_by_key(|(_, last_used, _)| *last_used);

            let mut evicted = Vec::new();
            for (key, _, estimated_bytes) in candidates {
                if loaded_bytes.saturating_add(requested_bytes) <= self.memory_budget_bytes {
                    break;
                }
                if let Some(slot) = state.slots.remove(&key) {
                    loaded_bytes = loaded_bytes.saturating_sub(estimated_bytes);
                    evicted.push(slot);
                }
            }
            (evicted, loaded_bytes)
        };

        if !evicted.is_empty() {
            self.loaded_count
                .fetch_sub(evicted.len(), Ordering::Relaxed);
            wait_for_model_drop(evicted).await;
        }
        if remaining_bytes.saturating_add(requested_bytes) > self.memory_budget_bytes {
            return Err(format!(
                "insufficient local model budget: {} bytes are resident and {} bytes are required",
                remaining_bytes, requested_bytes
            ));
        }
        Ok(())
    }

    fn next_access_sequence(&self) -> u64 {
        self.access_sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_add(1))
            })
            .unwrap_or(u64::MAX)
            .saturating_add(1)
    }
}

async fn wait_for_model_drop(evicted: Vec<Arc<ModelSlot>>) {
    let _ = tokio::task::spawn_blocking(move || drop(evicted)).await;
}

/// Gateway provider that resolves its llama.cpp model on the first request.
pub struct LazyLlamaGatewayProvider {
    id: String,
    config: Value,
    base_dir: PathBuf,
    pool: LocalModelPool,
}

impl LazyLlamaGatewayProvider {
    pub fn new(
        id: impl Into<String>,
        config: Value,
        base_dir: impl Into<PathBuf>,
        pool: LocalModelPool,
    ) -> Result<Self, String> {
        let id = id.into();
        let base_dir = base_dir.into();
        if id.trim().is_empty() {
            return Err("llama provider id must not be empty".to_string());
        }
        model_cache_key(&config, &base_dir).map(|cache_key| {
            pool.register_provider_model(&id, &cache_key);
            Self {
                id,
                config,
                base_dir,
                pool,
            }
        })
    }
}

impl AgentGatewayProvider for LazyLlamaGatewayProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn provider_type(&self) -> &str {
        "vifu-runtime"
    }

    fn invoke<'a>(
        &'a self,
        agent_id: &'a str,
        binding: &'a Value,
        input: &'a Value,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<Value, GatewayProviderError>> + Send + 'a>> {
        self.invoke_with_events(
            agent_id,
            binding,
            input,
            timeout,
            ProviderEventSink::discard(),
        )
    }

    fn invoke_with_events<'a>(
        &'a self,
        agent_id: &'a str,
        binding: &'a Value,
        input: &'a Value,
        timeout: Duration,
        events: ProviderEventSink,
    ) -> Pin<Box<dyn Future<Output = Result<Value, GatewayProviderError>> + Send + 'a>> {
        let cancellation = CancellationToken::default();
        let invocation = self.invoke_with_events_and_cancellation(
            agent_id,
            binding,
            input,
            timeout,
            cancellation.clone(),
            events,
        );
        Box::pin(async move {
            match tokio::time::timeout(timeout, invocation).await {
                Ok(response) => response,
                Err(_) => {
                    cancellation.cancel();
                    Err(GatewayProviderError::timed_out(
                        "local llama request timed out",
                    ))
                }
            }
        })
    }

    fn invoke_with_events_and_cancellation<'a>(
        &'a self,
        agent_id: &'a str,
        binding: &'a Value,
        input: &'a Value,
        timeout: Duration,
        cancellation: CancellationToken,
        events: ProviderEventSink,
    ) -> Pin<Box<dyn Future<Output = Result<Value, GatewayProviderError>> + Send + 'a>> {
        Box::pin(async move {
            let load_started = std::time::Instant::now();
            events.stage_started(vifu_gateway::relay::ProviderStage::Load, Value::Null);
            let load = self.pool.get_or_load(&self.config, &self.base_dir);
            tokio::pin!(load);
            let loaded = tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    return Err(GatewayProviderError::failed("local llama request cancelled"));
                }
                result = &mut load => result,
            };
            let (provider, was_resident) = match loaded {
                Ok(result) => result,
                Err(error) => {
                    events.stage_failed(
                        vifu_gateway::relay::ProviderStage::Load,
                        load_started
                            .elapsed()
                            .as_millis()
                            .try_into()
                            .unwrap_or(u64::MAX),
                        error.clone(),
                        Value::Null,
                    );
                    return Err(GatewayProviderError::failed(error));
                }
            };
            events.stage_completed(
                vifu_gateway::relay::ProviderStage::Load,
                load_started
                    .elapsed()
                    .as_millis()
                    .try_into()
                    .unwrap_or(u64::MAX),
                serde_json::json!({ "resident": was_resident }),
            );
            let gateway = InProcessGatewayProvider::new(self.id.clone(), provider)
                .map_err(GatewayProviderError::failed)?;
            gateway
                .invoke_with_events_and_cancellation(
                    agent_id,
                    binding,
                    input,
                    timeout,
                    cancellation,
                    events,
                )
                .await
        })
    }
}

pub fn llama_input_modalities(config: &Value) -> Value {
    if config
        .get("mmprojPath")
        .and_then(Value::as_str)
        .is_some_and(|path| !path.trim().is_empty())
    {
        serde_json::json!(["text", "image"])
    } else {
        serde_json::json!(["text"])
    }
}

fn model_cache_key(config: &Value, base_dir: &Path) -> Result<String, String> {
    let model_path = configured_model_path(config, base_dir)?;
    let normalized_base = base_dir
        .canonicalize()
        .unwrap_or_else(|_| base_dir.to_path_buf());
    let encoded = serde_json::to_string(config)
        .map_err(|error| format!("llama provider configuration could not be encoded: {error}"))?;
    Ok(format!(
        "{}\n{}\n{encoded}",
        normalized_base.display(),
        model_path.display()
    ))
}

fn configured_model_path(config: &Value, base_dir: &Path) -> Result<PathBuf, String> {
    let value = config
        .get("modelPath")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "llama provider requires config.modelPath".to_string())?;
    let path = PathBuf::from(value);
    Ok(if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    })
}

fn configured_model_bytes(config: &Value, base_dir: &Path) -> u64 {
    let weights = configured_model_path(config, base_dir)
        .ok()
        .and_then(|path| path.metadata().ok())
        .map_or(0, |metadata| metadata.len());
    let projector = config
        .get("mmprojPath")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                base_dir.join(path)
            }
        })
        .and_then(|path| path.metadata().ok())
        .map_or(0, |metadata| metadata.len());
    let context_size = config
        .get("contextSize")
        .and_then(Value::as_u64)
        .unwrap_or(4_096);
    let max_concurrency = config
        .get("maxConcurrency")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1);
    // Admission estimate, not a reported metric: weights/projector plus a
    // conservative context/scratch reserve. UI memory evidence remains sampled
    // OS process RSS, because backend/GPU allocation differs by device.
    let context_and_scratch = context_size
        .saturating_mul(64 * 1024)
        .saturating_mul(max_concurrency)
        .max(weights / 4)
        .max(128 * 1024 * 1024);
    weights
        .saturating_add(projector)
        .saturating_add(context_and_scratch)
}

#[cfg(target_os = "linux")]
fn physical_memory_bytes() -> Option<u64> {
    // Values are queried once when the pool is created.
    let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    (pages > 0 && page_size > 0).then(|| (pages as u64).saturating_mul(page_size as u64))
}

#[cfg(target_os = "macos")]
fn physical_memory_bytes() -> Option<u64> {
    let mut value = 0_u64;
    let mut size = std::mem::size_of::<u64>();
    let name = std::ffi::CString::new("hw.memsize").ok()?;
    // SAFETY: `value` and `size` are valid writable buffers and the name is NUL-terminated.
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::addr_of_mut!(value).cast(),
            std::ptr::addr_of_mut!(size),
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0).then_some(value)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn physical_memory_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use super::{llama_input_modalities, LazyLlamaGatewayProvider, LocalModelPool};
    use serde_json::json;

    #[tokio::test]
    async fn configured_models_are_not_loaded_by_registration() {
        let pool = LocalModelPool::with_memory_budget(1024);
        let _provider = LazyLlamaGatewayProvider::new(
            "planner",
            json!({"modelPath": "missing.gguf"}),
            "/tmp/vifu-lazy-model-test",
            pool.clone(),
        )
        .unwrap();

        assert_eq!(pool.loaded_count(), 0);
    }

    #[test]
    fn current_route_model_is_protected_from_budget_eviction() {
        let pool = LocalModelPool::with_memory_budget(1024);
        let base_dir = std::path::Path::new("/tmp/vifu-route-residency-test");
        let current_config = json!({"modelPath": "current.gguf"});
        let candidate_config = json!({"modelPath": "candidate.gguf"});
        let current_key = super::model_cache_key(&current_config, base_dir).unwrap();
        let candidate_key = super::model_cache_key(&candidate_config, base_dir).unwrap();
        let _current = LazyLlamaGatewayProvider::new(
            "current-provider",
            current_config,
            base_dir,
            pool.clone(),
        )
        .unwrap();
        let _candidate = LazyLlamaGatewayProvider::new(
            "candidate-provider",
            candidate_config,
            base_dir,
            pool.clone(),
        )
        .unwrap();

        pool.set_active_route("route-a", "current-provider");
        let protected = pool.protected_cache_keys();
        assert!(protected.contains(&current_key));
        assert!(!protected.contains(&candidate_key));

        pool.set_active_route("route-a", "candidate-provider");
        let protected = pool.protected_cache_keys();
        assert!(!protected.contains(&current_key));
        assert!(protected.contains(&candidate_key));

        pool.clear_active_routes();
        assert!(pool.protected_cache_keys().is_empty());
    }

    #[tokio::test]
    async fn admission_rejects_a_model_larger_than_the_hard_budget() {
        let pool = LocalModelPool::with_memory_budget(128);

        let error = pool.evict_for_owned("oversized", 129).await.unwrap_err();

        assert!(error.contains("exceeds the local model budget"));
        assert_eq!(pool.loaded_count(), 0);
    }

    #[tokio::test]
    async fn cancelled_waiter_cannot_start_an_overlapping_model_load() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let attempts = Arc::new(AtomicUsize::new(0));
        let loader = {
            let active = Arc::clone(&active);
            let max_active = Arc::clone(&max_active);
            let attempts = Arc::clone(&attempts);
            Arc::new(move |_: serde_json::Value, _: std::path::PathBuf| {
                attempts.fetch_add(1, Ordering::SeqCst);
                let active_now = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_active.fetch_max(active_now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(50));
                active.fetch_sub(1, Ordering::SeqCst);
                Err("injected load failure".to_string())
            })
        };
        let pool = LocalModelPool::with_test_loader(1024 * 1024 * 1024, loader);
        let first_pool = pool.clone();
        let first = tokio::spawn(async move {
            tokio::time::timeout(
                Duration::from_millis(5),
                first_pool.get_or_load(
                    &json!({"modelPath": "cancelled.gguf"}),
                    std::path::Path::new("/tmp"),
                ),
            )
            .await
        });
        while attempts.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }

        assert!(first.await.unwrap().is_err());
        let second = pool
            .get_or_load(
                &json!({"modelPath": "cancelled.gguf"}),
                std::path::Path::new("/tmp"),
            )
            .await;

        let Err(error) = second else {
            panic!("injected loader unexpectedly returned a model");
        };
        assert_eq!(error, "injected load failure");
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn multimodal_configuration_is_visible_before_model_load() {
        assert_eq!(
            llama_input_modalities(&json!({
                "modelPath": "vision.gguf",
                "mmprojPath": "vision-mmproj.gguf"
            })),
            json!(["text", "image"])
        );
        assert_eq!(
            llama_input_modalities(&json!({"modelPath": "chat.gguf"})),
            json!(["text"])
        );
    }

    #[test]
    fn provider_requires_a_model_path_but_not_a_present_model_file() {
        let pool = LocalModelPool::with_memory_budget(1024);
        let error = LazyLlamaGatewayProvider::new("planner", json!({}), "/tmp", pool)
            .err()
            .unwrap();

        assert_eq!(error, "llama provider requires config.modelPath");
    }
}
