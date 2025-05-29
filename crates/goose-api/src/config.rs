use crate::handlers::AGENT;
use goose::config::{Config, ExtensionEntry};
use goose::agents::ExtensionConfig;
use goose::providers::{create, providers};
use goose::model::ModelConfig;
use tracing::{info, warn, error};
use config::{builder::DefaultState, ConfigBuilder, Environment, File};
use serde_json::Value;

pub fn load_configuration() -> std::result::Result<config::Config, config::ConfigError> {
    let config_path = std::env::var("GOOSE_CONFIG").unwrap_or_else(|_| "config".to_string());
    let builder = ConfigBuilder::<DefaultState>::default()
        .add_source(File::with_name(&config_path).required(false))
        .add_source(Environment::with_prefix("GOOSE_API"));
    builder.build()
}

pub async fn initialize_provider_config() -> Result<(), anyhow::Error> {
    let api_config = load_configuration()?;

    let provider_name = std::env::var("GOOSE_API_PROVIDER")
        .or_else(|_| api_config.get_string("provider"))
        .unwrap_or_else(|_| "openai".to_string());

    let model_name = std::env::var("GOOSE_API_MODEL")
        .or_else(|_| api_config.get_string("model"))
        .unwrap_or_else(|_| "gpt-4o".to_string());

    info!("Initializing with provider: {}, model: {}", provider_name, model_name);

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
            } else {
                warn!("Environment variable not set for key: {}", key.name);
                if key.required {
                    error!("Required key {} not provided", key.name);
                    return Err(anyhow::anyhow!("Required key {} not provided", key.name));
                }
            }
        }
    }

    let model_config = ModelConfig::new(model_name);
    let provider = create(&provider_name, model_config)?;

    let agent = AGENT.lock().await;
    agent.update_provider(provider).await?;

    info!("Provider configuration successful");
    Ok(())
}

pub async fn initialize_extensions(config: &config::Config) -> Result<(), anyhow::Error> {
    if let Ok(ext_table) = config.get_table("extensions") {
        for (name, ext_config) in ext_table {
            let entry: ExtensionEntry = ext_config.clone().try_deserialize()
                .map_err(|e| anyhow::anyhow!("Failed to deserialize extension config for {}: {}", name, e))?;

            if entry.enabled {
                let extension_config: ExtensionConfig = entry.config;
                let agent = AGENT.lock().await;
                if let Err(e) = agent.add_extension(extension_config).await {
                    error!("Failed to add extension {}: {}", name, e);
                }
            } else {
                info!("Skipping disabled extension: {}", name);
            }
        }
    } else {
        warn!("No extensions configured in config file.");
    }
    Ok(())
}

pub async fn run_init_tests() -> Result<(), anyhow::Error> {
    info!("Running initialization tests");
    {
        let _agent = AGENT.lock().await;
        info!("Agent initialization test passed");
    }
    info!("Initialization tests completed");
    Ok(())
}
