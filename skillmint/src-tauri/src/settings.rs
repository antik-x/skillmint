use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::acp::AcpTransport;
use crate::models::{SkillScopeMode, SyncMode};

/// PRD-08: supported LLM providers for classification + daily summaries.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
pub enum AiProvider {
    /// OpenAI-compatible chat/completions endpoint (default).
    #[default]
    #[serde(rename = "openai")]
    OpenAI,
    /// Anthropic Messages API.
    #[serde(rename = "anthropic")]
    Anthropic,
}

impl<'de> serde::Deserialize<'de> for AiProvider {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct AiProviderVisitor;
        impl<'de> serde::de::Visitor<'de> for AiProviderVisitor {
            type Value = AiProvider;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("'openai' or 'anthropic'")
            }
            fn visit_str<E>(self, value: &str) -> std::result::Result<AiProvider, E>
            where
                E: serde::de::Error,
            {
                Ok(AiProvider::from_str(value))
            }
        }
        deserializer.deserialize_str(AiProviderVisitor)
    }
}

impl std::fmt::Display for AiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiProvider::OpenAI => write!(f, "openai"),
            AiProvider::Anthropic => write!(f, "anthropic"),
        }
    }
}

impl AiProvider {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "anthropic" => AiProvider::Anthropic,
            _ => AiProvider::OpenAI,
        }
    }
}

/// PRD-08: billing mode for pricing/cost attribution.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BillingMode {
    /// Pay-as-you-go (variable cost, real money per token).
    #[default]
    PayAsYouGo,
    /// Subscription / flat-rate (no per-token variable cost).
    Subscription,
}

impl std::fmt::Display for BillingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BillingMode::PayAsYouGo => write!(f, "pay_as_you_go"),
            BillingMode::Subscription => write!(f, "subscription"),
        }
    }
}

impl BillingMode {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "subscription" => BillingMode::Subscription,
            _ => BillingMode::PayAsYouGo,
        }
    }
}

const SETTINGS_FILE: &str = "settings.json";
/// Keyring service name for SkillMint credentials.
const KEYRING_SERVICE: &str = "com.skillmint";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub device_id: String,
    /// Missing in settings.json written by older versions; an empty path is
    /// normalized to `~/.skillmint/repo` in `load_or_default`.
    #[serde(default)]
    pub center_repo: PathBuf,
    #[serde(default = "default_sync_mode")]
    pub default_sync_mode: SyncMode,
    #[serde(default = "default_auto_sync_interval")]
    pub auto_sync_interval_minutes: u32,
    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default = "default_show_dock_icon")]
    pub show_dock_icon: bool,
    #[serde(default)]
    pub onboarding_completed: bool,
    /// P0-3: user-configured extra directory names excluded from skill
    /// scan/import, on top of the built-in `scan::DEFAULT_SCAN_EXCLUSIONS`.
    #[serde(default)]
    pub scan_exclude_names: Vec<String>,
    #[serde(default)]
    pub skill_scope_mode: SkillScopeMode,
    #[serde(default = "default_project_skill_dir")]
    pub project_skill_dir_name: String,
    /// PRD-07: master switch for all remote (network) features. Default false —
    /// local-first guarantee; must be explicitly enabled by the user.
    #[serde(default)]
    pub remote_enabled: bool,
    /// PRD-08 §3.6: optional LLM config for prompt semantic classification and
    /// daily summaries. When `api_key` is empty, LLM features are skipped and
    /// the pipeline never hard-fails (graceful degradation).
    ///
    /// SECURITY: `api_key` is NEVER serialized to settings.json. It is read from
    /// and written to the platform credential store (macOS Keychain) on load/save.
    #[serde(default)]
    pub ai: AiConfig,
}

/// One configured LLM endpoint. Supports chat and/or embedding capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiModelConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub provider: AiProvider,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub base_url: String,
    /// API key is stored in the keyring, not settings.json. In-memory only.
    #[serde(default, skip_serializing)]
    pub api_key: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// One configured ACP connection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AcpConnectionConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub transport: AcpTransport,
}

/// PRD-08: optional AI/LLM configuration (OpenAI-compatible or Anthropic).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default)]
    pub models: Vec<AiModelConfig>,
    #[serde(default)]
    pub acp_connections: Vec<AcpConnectionConfig>,
    #[serde(default)]
    pub default_chat_model_id: Option<String>,
    #[serde(default)]
    pub default_embedding_model_id: Option<String>,
    #[serde(default)]
    pub prefer_acp: bool,
    /// P0: when true, LLM analysis never falls back to cloud providers;
    /// local ACP agents are the only allowed path.
    #[serde(default)]
    pub strict_local_mode: bool,
}

/// Legacy single-model + single-ACP-block config, used for migration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LegacyAiConfig {
    #[serde(default, skip_serializing)]
    api_key: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    provider: AiProvider,
    #[serde(default)]
    acp: LegacyAcpConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LegacyAcpConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    transports: Vec<AcpTransport>,
}

impl AiConfig {
    fn keyring_username(model_id: &str) -> String {
        format!("model:{}", model_id)
    }

    /// Read an API key for a specific model from the keyring.
    pub fn read_key(model_id: &str) -> String {
        match keyring::Entry::new(KEYRING_SERVICE, &Self::keyring_username(model_id)) {
            Ok(entry) => entry.get_password().unwrap_or_default(),
            Err(e) => {
                eprintln!("[keyring] failed to open entry for {}: {}", model_id, e);
                String::new()
            }
        }
    }

    /// Persist an API key for a specific model to the keyring.
    pub fn write_key(model_id: &str, key: &str) {
        if key.trim().is_empty() {
            let _ = Self::delete_key(model_id);
            return;
        }
        match keyring::Entry::new(KEYRING_SERVICE, &Self::keyring_username(model_id)) {
            Ok(entry) => {
                if let Err(e) = entry.set_password(key) {
                    eprintln!("[keyring] failed to store API key for {}: {}", model_id, e);
                }
            }
            Err(e) => eprintln!("[keyring] failed to open entry for {}: {}", model_id, e),
        }
    }

    /// Remove an API key for a specific model from the keyring.
    pub fn delete_key(model_id: &str) -> Result<()> {
        match keyring::Entry::new(KEYRING_SERVICE, &Self::keyring_username(model_id)) {
            Ok(entry) => entry.delete_credential().map_err(|e| e.into()),
            Err(e) => Err(anyhow::anyhow!("failed to open keyring entry for {}: {}", model_id, e)),
        }
    }

    /// Migrate the legacy single-model config into the new list format.
    fn from_legacy(legacy: LegacyAiConfig) -> Self {
        let mut models = Vec::new();
        let has_any_config = !legacy.model.trim().is_empty()
            || !legacy.base_url.trim().is_empty()
            || !legacy.api_key.trim().is_empty();
        let default_id = "default".to_string();
        if has_any_config {
            models.push(AiModelConfig {
                id: default_id.clone(),
                name: if legacy.model.trim().is_empty() {
                    "默认模型".to_string()
                } else {
                    legacy.model.clone()
                },
                provider: legacy.provider,
                model: legacy.model,
                base_url: legacy.base_url,
                api_key: legacy.api_key,
                capabilities: vec!["chat".to_string()],
            });
        }

        let acp_connections: Vec<AcpConnectionConfig> = legacy
            .acp
            .transports
            .into_iter()
            .enumerate()
            .map(|(idx, transport)| AcpConnectionConfig {
                id: format!("acp-{}", idx),
                name: format!("ACP 连接 {}", idx + 1),
                enabled: legacy.acp.enabled,
                transport,
            })
            .collect();

        Self {
            models,
            acp_connections,
            default_chat_model_id: if has_any_config { Some(default_id) } else { None },
            default_embedding_model_id: None,
            prefer_acp: false,
            strict_local_mode: false,
        }
    }

    /// Load keys for all models from the keyring.
    pub fn load_keys(&mut self) {
        for model in &mut self.models {
            model.api_key = Self::read_key(&model.id);
        }
    }

    /// Persist keys for all models to the keyring and clear them from memory
    /// so they are not written to settings.json.
    pub fn persist_keys_and_sanitize(&mut self) {
        for model in &mut self.models {
            Self::write_key(&model.id, &model.api_key);
            model.api_key.clear();
        }
    }
}

fn default_project_skill_dir() -> String {
    ".skillmint/skills".to_string()
}

// Serde defaults for fields that may be missing in settings.json written by
// older app versions. Values must stay in sync with `Default for Settings`.
fn default_sync_mode() -> SyncMode {
    SyncMode::Symlink
}

fn default_auto_sync_interval() -> u32 {
    5
}

fn default_show_dock_icon() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            device_id: Uuid::new_v4().to_string(),
            center_repo: home.join(".skillmint").join("repo"),
            default_sync_mode: SyncMode::Symlink,
            auto_sync_interval_minutes: 5,
            launch_at_login: false,
            show_dock_icon: true,
            onboarding_completed: false,
            scan_exclude_names: Vec::new(),
            skill_scope_mode: SkillScopeMode::Global,
            project_skill_dir_name: default_project_skill_dir(),
            remote_enabled: false,
            ai: AiConfig::default(),
        }
    }
}

impl Settings {
    pub fn load_or_default(app_dir: &PathBuf) -> Result<Self> {
        let path = app_dir.join(SETTINGS_FILE);
        let mut settings = if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let mut raw: serde_json::Value = serde_json::from_str(&content)?;

            // Migrate legacy AI config (single model + single ACP block) to the
            // new list-based format before deserializing.
            if let Some(ai_value) = raw.get_mut("ai") {
                if ai_value.get("api_key").is_some() || ai_value.get("model").is_some() {
                    if let Ok(legacy) = serde_json::from_value::<LegacyAiConfig>(ai_value.take()) {
                        let migrated = AiConfig::from_legacy(legacy);
                        *ai_value = serde_json::to_value(&migrated)?;
                    }
                }
            }

            let mut s: Settings = serde_json::from_value(raw)?;
            s.center_repo = crate::scan::expand_path(&s.center_repo.to_string_lossy());
            if s.center_repo.as_os_str().is_empty() {
                s.center_repo = dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".skillmint")
                    .join("repo");
            }
            s
        } else {
            Settings::default()
        };

        // Guarantee a stable device_id: regenerate + persist only when missing/empty.
        let mut dirty = false;
        if settings.device_id.trim().is_empty() {
            settings.device_id = Uuid::new_v4().to_string();
            dirty = true;
        }

        // Load keys for all models from the keyring into memory.
        settings.ai.load_keys();

        // Always persist on first run or when we regenerated the device_id.
        if dirty || !path.exists() {
            settings.save(app_dir)?;
        }
        Ok(settings)
    }

    /// Save settings to disk. API keys are NOT written to settings.json; they
    /// are persisted separately to the platform credential store.
    pub fn save(&self, app_dir: &PathBuf) -> Result<()> {
        let path = app_dir.join(SETTINGS_FILE);
        // Persist keys and ensure in-memory api_keys never reach the disk file.
        let mut sanitized = self.clone();
        sanitized.ai.persist_keys_and_sanitize();
        let content = serde_json::to_string_pretty(&sanitized)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Persist all in-memory model API keys to the keyring.
    pub fn persist_api_keys(&self) {
        let mut copy = self.ai.clone();
        copy.persist_keys_and_sanitize();
    }
}
