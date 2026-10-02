//! Default model configuration.
//!
//! No built-in default model is provided. The user must configure a model
//! in `config.toml` or via CLI flags / environment variables. If no model
//! is configured, the application errors at startup.
//!
//! At runtime each model is resolved via:
//!   CLI flag > ENV var > config.toml > remote settings

use std::sync::LazyLock;

/// The raw JSON, embedded at compile time. Re-exported through the
/// `codel_shell::models` facade and consumed by `agent::config`, so it must
/// be `pub`.
pub const DEFAULT_MODELS_JSON: &str = include_str!("../default_models.json");

#[derive(serde::Deserialize)]
struct DefaultModels {
    /// Primary model ID. `None` means no built-in default — user must configure.
    #[serde(default)]
    default: Option<String>,
    /// Falls back to `default` if not specified in JSON.
    #[serde(default)]
    web_search: Option<String>,
    /// Falls back to `default` if not specified in JSON.
    #[serde(default)]
    image_description: Option<String>,
    /// Falls back to `default` if not specified in JSON.
    #[serde(default)]
    session_summary: Option<String>,
}

static DEFAULTS: LazyLock<DefaultModels> = LazyLock::new(|| {
    serde_json::from_str(DEFAULT_MODELS_JSON).expect("default_models.json: invalid JSON")
});

/// Primary model for coding tasks.
///
/// Empty when no built-in default is configured: the fork ships an empty
/// catalogue, so the model must come from `config.toml`, a CLI flag, or the
/// environment. Callers treat the empty string as "unset".
pub fn default_model() -> &'static str {
    DEFAULTS.default.as_deref().unwrap_or("")
}

/// Model for web search tool synthesis. Falls back to the default model.
pub fn default_web_search_model() -> &'static str {
    DEFAULTS
        .web_search
        .as_deref()
        .or(DEFAULTS.default.as_deref())
        .unwrap_or("")
}

/// Model for image describe. Falls back to the default model.
pub fn default_image_description_model() -> &'static str {
    DEFAULTS
        .image_description
        .as_deref()
        .or(DEFAULTS.default.as_deref())
        .unwrap_or("")
}

/// Model for session title generation. Falls back to the default model.
pub fn default_session_summary_model() -> &'static str {
    DEFAULTS
        .session_summary
        .as_deref()
        .or(DEFAULTS.default.as_deref())
        .unwrap_or("")
}
