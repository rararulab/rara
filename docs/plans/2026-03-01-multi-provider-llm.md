# Multi-Provider LLM Architecture

## Problem

Current system supports only one globally active LLM provider at a time. Users cannot:
- Run different agents on different providers simultaneously (e.g., rara on OpenRouter, scout on Ollama)
- Override model selection per-agent at runtime without editing code
- Have multiple providers active concurrently

## Design

### Core Concept: ProviderRegistry

Replace `LlmProviderLoader` (single provider) with `ProviderRegistry` (named provider map + resolution logic).

```rust
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    default_provider: String,
    default_model: String,
    agent_overrides: HashMap<String, AgentLlmConfig>,
}

pub struct AgentLlmConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
}
```

### Resolution Priority

```
Provider: agent settings > manifest provider_hint > global default
Model:    agent settings > manifest model > global default
```

`ProviderRegistry::resolve(agent_name, manifest) -> Result<(Arc<dyn LlmProvider>, String)>`

### Settings Keys (DB KV)

```
# Global defaults
llm.default_provider = "openrouter"
llm.default_model = "openai/gpt-4o-mini"

# Provider configs (each independently enabled)
llm.providers.openrouter.enabled = true
llm.providers.openrouter.api_key = "sk-..."
llm.providers.openrouter.base_url = "https://openrouter.ai/api/v1"

llm.providers.ollama.enabled = true
llm.providers.ollama.base_url = "http://localhost:11434"

llm.providers.codex.enabled = true

# Per-agent overrides (optional)
llm.agents.rara.provider = "ollama"
llm.agents.rara.model = "qwen3:32b"
```

### AgentManifest Changes

```rust
// Before
pub model: String,
pub provider_hint: Option<String>,

// After
pub model: Option<String>,         // default from manifest, overridable
pub provider_hint: Option<String>, // kept as manifest-level fallback
```

## Code Changes

### 1. ProviderRegistry (kernel/src/provider/)

New `registry.rs`:
- `ProviderRegistry` struct with HashMap + defaults + agent overrides
- `resolve(agent_name, manifest) -> Result<(Arc<dyn LlmProvider>, String)>`
- `reload_from_settings(settings)` — refresh from DB without restart

### 2. Delete LlmProviderLoader trait

- Remove `LlmProviderLoader` and `LlmProviderLoaderRef` from `provider/mod.rs`
- Remove `EnvLlmProviderLoader`, `OllamaProviderLoader` from `provider/mod.rs`
- Remove `SettingsLlmProviderLoader` from `workers/worker_state.rs`

### 3. AgentManifest update

- `model: String` → `model: Option<String>` in `process/mod.rs`
- Update `rara()` manifest in `agents/src/lib.rs` — set model to `None`
- Update YAML deserialization if any

### 4. agent_turn.rs

- Current: `acquire_provider()` syscall → provider, then uses `manifest.model`
- New: `resolve_provider()` syscall → `(provider, model_name)` pair
- Pass resolved model name to `CreateChatCompletionRequestArgs::default().model(model_name)`

### 5. Syscall + Event Loop

- Replace `Syscall::AcquireProvider` with `Syscall::ResolveProvider`
- Handler calls `registry.resolve(agent_name, manifest)`
- Returns `(Arc<dyn LlmProvider>, String)` tuple

### 6. KernelInner

- Replace `llm_provider: LlmProviderLoaderRef` with `provider_registry: Arc<ProviderRegistry>`

### 7. Boot layer

- `BootConfig` takes `provider_registry: Arc<ProviderRegistry>` instead of `llm_provider`
- `workers/worker_state.rs`: build ProviderRegistry from settings, pass to boot

### 8. Settings keys module

- Add new keys to `shared/src/settings/mod.rs`

## Testing

- Unit tests: registry resolution with various override combinations
- Unit tests: missing provider, disabled provider error cases
- Existing kernel tests: update to use ProviderRegistry with test providers
