//! Command line access to Brain, providers, and hardware.

use crate::tui::{self, App};
use anyhow::{anyhow, Result};
use argos_osint_core::brain::MemorySource;
use argos_osint_core::hardware;
use argos_osint_core::paths;
use argos_osint_core::provider::{self, SettingsFile};
use argos_osint_core::secrets::{AuthFile, ProviderSecret};
use argos_osint_core::store::Store;
use clap::{Parser, Subcommand};
use std::io::{self, Write};

#[derive(Parser)]
#[command(name = "argos", version, about = "Argos memory and provider shell")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Save a sourced insight for recall by another chat app.
    Remember {
        #[arg(long)]
        app: String,
        #[arg(long)]
        conversation: String,
        #[arg(long)]
        message: Option<String>,
        #[arg(long)]
        reference: Option<String>,
        #[arg(long, default_value = "fact")]
        category: String,
        text: String,
    },
    /// Retrieve relevant saved insights as JSON, including their source metadata.
    Recall {
        query: String,
        #[arg(long, default_value_t = 8)]
        limit: usize,
    },
    /// List all saved memories as JSON.
    Memories,
    /// Sign in or configure a provider account.
    Login,
    /// Forget saved provider credentials.
    Logout,
    /// Print the host profile.
    Hardware,
    /// List the active text model catalog.
    Models,
}

pub async fn dispatch() -> Result<()> {
    match Cli::parse().command {
        None => tui::run(App::boot()?).await,
        Some(Command::Remember {
            app,
            conversation,
            message,
            reference,
            category,
            text,
        }) => {
            let store = open_store()?;
            let memory = store.add_memory(
                &text,
                &category,
                false,
                MemorySource {
                    app,
                    conversation_id: conversation,
                    message_id: message,
                    reference,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&memory)?);
            Ok(())
        }
        Some(Command::Recall { query, limit }) => {
            let hits = open_store()?.recall(&query, limit)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &hits
                        .iter()
                        .map(|hit| serde_json::json!({"score":hit.score,"memory":hit.memory}))
                        .collect::<Vec<_>>()
                )?
            );
            Ok(())
        }
        Some(Command::Memories) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&open_store()?.list_memories()?)?
            );
            Ok(())
        }
        Some(Command::Login) => login().await,
        Some(Command::Logout) => logout(),
        Some(Command::Hardware) => {
            let p = hardware::profile_cached(false);
            println!("{}", p.one_line());
            Ok(())
        }
        Some(Command::Models) => models().await,
    }
}

fn open_store() -> Result<Store> {
    paths::ensure_home()?;
    Store::open(&paths::db_path())
}

fn ask(label: &str, default: &str) -> Result<String> {
    if default.is_empty() {
        print!("{label}: ");
    } else {
        print!("{label} [{default}]: ");
    }
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let value = line.trim();
    Ok(if value.is_empty() {
        default.into()
    } else {
        value.into()
    })
}

async fn login() -> Result<()> {
    let kind = ask("Provider (grok/openai/openrouter/local)", "grok")?;
    let kind = provider::normalize_kind(&kind);
    if kind == "grok" {
        println!(
            "{}",
            argos_osint_core::grok_oauth::login(|line| println!("{line}")).await?
        );
        return Ok(());
    }
    if matches!(kind.as_str(), "openai" | "openai-chatgpt") {
        println!(
            "{}",
            argos_osint_core::subscription::login(|line| println!("{line}")).await?
        );
        return Ok(());
    }
    let preset = provider::preset(&kind).ok_or_else(|| anyhow!("unknown provider {kind}"))?;
    let base_url = ask("Base URL", preset.base_url)?;
    let model = ask("Model", preset.text_model)?;
    let api_key =
        rpassword::prompt_password("API key (empty for local server or environment variable): ")?;
    let secret = ProviderSecret {
        kind: kind.clone(),
        base_url,
        model: model.clone(),
        api_key: if api_key.trim().is_empty() {
            None
        } else {
            Some(api_key.trim().into())
        },
        stt_model: None,
        device: None,
    };
    let mut auth = AuthFile::load()?;
    auth.set_account(secret);
    auth.save()?;
    let mut settings = SettingsFile::load()?;
    settings.writer_provider = kind;
    settings.writer_model = model;
    settings.save()?;
    println!("Provider and writer model saved.");
    Ok(())
}

fn logout() -> Result<()> {
    let mut auth = AuthFile::load()?;
    auth.accounts.clear();
    auth.text = None;
    auth.voice = None;
    auth.save()?;
    println!("Saved Argos provider credentials removed. Subscription CLI sessions remain managed by their own tools.");
    Ok(())
}

async fn models() -> Result<()> {
    let auth = AuthFile::load()?;
    let settings = SettingsFile::load()?;
    let secret = provider::writer_secret(&auth, &settings);
    println!("{} / {}", provider::effective_kind(&secret), secret.model);
    match provider::list_models(&secret).await {
        Ok(models) => {
            for model in models {
                println!("{model}");
            }
        }
        Err(err) => eprintln!("catalog unavailable: {err}"),
    }
    Ok(())
}
