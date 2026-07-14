//! API-key storage backed by the operating system credential vault.

use anyhow::{Context, Result};

use crate::cli::Provider;

const SERVICE: &str = "zavora-cli";

fn account(provider: Provider) -> Result<&'static str> {
    match provider {
        Provider::Openai => Ok("openai"),
        Provider::Anthropic => Ok("anthropic"),
        Provider::Gemini => Ok("gemini"),
        Provider::Deepseek => Ok("deepseek"),
        Provider::Groq => Ok("groq"),
        Provider::Ollama => anyhow::bail!("Ollama does not use an API key"),
        Provider::Auto => anyhow::bail!("resolve a provider before loading credentials"),
    }
}

pub fn store_api_key(provider: Provider, key: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, account(provider)?)
        .context("could not open the operating system credential vault")?;
    entry
        .set_password(key)
        .context("could not save the API key in the operating system credential vault")
}

pub fn load_api_key(provider: Provider) -> Option<String> {
    let account = account(provider).ok()?;
    let entry = keyring::Entry::new(SERVICE, account).ok()?;
    entry
        .get_password()
        .ok()
        .filter(|key| !key.trim().is_empty())
}
