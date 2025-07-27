use goose::config::{Config, ExtensionEntry};
use goose::agents::ExtensionConfig;
use goose::providers::providers;
use tracing::{info, warn, error};
use config::{builder::DefaultState, ConfigBuilder, Environment, File};
use serde_json::Value;
use std::sync::{Arc, LazyLock};
use tokio::sync::RwLock;

pub static PROVIDER_CONFIG: LazyLock<Arc<RwLock<Option<ProviderConfig>>>> = LazyLock::new(|| {
    Arc::new(RwLock::new(None))
});

pub fn load_configuration() -> std::result::Result<config::Config, config::ConfigError> {
    // Determine the configuration file based on priority:
    // 1. Explicit GOOSE_CONFIG env var
    // 2. Goose CLI config if it exists
    // 3. Fallback to config file packaged with goose-api

    let config_path = if let Ok(path) = std::env::var("GOOSE_CONFIG") {
        path
    } else {
        let global = Config::global();
        if global.exists() {
            global.path()
        } else {
            // Use the config file that ships with goose-api
            format!("{}/config", env!("CARGO_MANIFEST_DIR"))
        }
    };

    let builder = ConfigBuilder::<DefaultState>::default()
        .add_source(File::with_name(&config_path).required(false))
        .add_source(Environment::with_prefix("GOOSE_API"));
    builder.build()
}

#[derive(Clone, Debug)]
pub struct ProviderConfig {
    pub provider_name: String,
    pub model_name: String,
}

pub async fn load_provider_config() -> Result<ProviderConfig, anyhow::Error> {
    eprintln!("[DEBUG] Starting initialize_provider_config");
    let api_config = load_configuration()?;

    let global_config = Config::global();

    let provider_name = if let Ok(val) = std::env::var("GOOSE_API_PROVIDER") {
        val
    } else if let Ok(val) = api_config.get_string("provider") {
        val
    } else if global_config.exists() {
        global_config
            .get_param::<String>("GOOSE_PROVIDER")
            .unwrap_or_else(|_| "openai".to_string())
    } else {
        "openai".to_string()
    };

    let model_name = if let Ok(val) = std::env::var("GOOSE_API_MODEL") {
        val
    } else if let Ok(val) = api_config.get_string("model") {
        val
    } else if global_config.exists() {
        global_config
            .get_param::<String>("GOOSE_MODEL")
            .unwrap_or_else(|_| "gpt-4o".to_string())
    } else {
        "gpt-4o".to_string()
    };

    info!("Initializing with provider: {}, model: {}", provider_name, model_name);
    eprintln!("[DEBUG] Provider: {}, Model: {}", provider_name, model_name);

    let config = Config::global();
    config.set_param("GOOSE_PROVIDER", Value::String(provider_name.clone()))?;
    config.set_param("GOOSE_MODEL", Value::String(model_name.clone()))?;

    let available_providers = providers();
    if let Some(provider_meta) = available_providers.iter().find(|p| p.name == provider_name) {
        for key in &provider_meta.config_keys {
            let env_name = key.name.clone();
            if let Ok(value) = std::env::var(&env_name) {
                if key.secret {
                    config.set_secret(&key.name, Value::String(value))?;
                    info!("Set secret key: {}", key.name);
                } else {
                    config.set_param(&key.name, Value::String(value))?;
                    info!("Set parameter: {}", key.name);
                }
            } else if global_config.exists() {
                // If not provided via environment, try existing CLI config
                let result: Result<String, _> = if key.secret {
                    global_config.get_secret(&key.name)
                } else {
                    global_config.get_param(&key.name)
                };

                match result {
                    Ok(value) => {
                        if key.secret {
                            config.set_secret(&key.name, Value::String(value))?;
                        } else {
                            config.set_param(&key.name, Value::String(value))?;
                        }
                        info!("Loaded {} from CLI config", key.name);
                    }
                    Err(_) => {
                        if let Some(default) = &key.default {
                            if key.secret {
                                config.set_secret(&key.name, Value::String(default.clone()))?;
                            } else {
                                config.set_param(&key.name, Value::String(default.clone()))?;
                            }
                            info!("Using default for {}", key.name);
                        } else if key.required {
                            error!("Required key {} not provided", key.name);
                            return Err(anyhow::anyhow!("Required key {} not provided", key.name));
                        } else {
                            warn!("Environment variable not set for key: {}", key.name);
                        }
                    }
                }
            } else if let Some(default) = &key.default {
                if key.secret {
                    config.set_secret(&key.name, Value::String(default.clone()))?;
                } else {
                    config.set_param(&key.name, Value::String(default.clone()))?;
                }
                info!("Using default for {}", key.name);

            } else if key.required {
                error!("Required key {} not provided", key.name);
                return Err(anyhow::anyhow!("Required key {} not provided", key.name));
            }
        }
    }

    Ok(ProviderConfig {
        provider_name: provider_name.clone(),
        model_name: model_name.clone(),
    })
}

pub fn load_extensions_config(config: &config::Config) -> Vec<(String, ExtensionConfig)> {
    let mut extensions = Vec::new();
    
    if let Ok(ext_table) = config.get_table("extensions") {
        for (name, ext_config) in ext_table {
            match ext_config.clone().try_deserialize::<ExtensionEntry>() {
                Ok(entry) if entry.enabled => {
                    extensions.push((name.clone(), entry.config));
                }
                Ok(_) => info!("Skipping disabled extension: {}", name),
                Err(e) => error!("Failed to deserialize extension config for {}: {}", name, e),
            }
        }
    } else {
        warn!("No extensions configured in config file.");
    }
    
    extensions
}

