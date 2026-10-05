//! Command line access to Brain, providers, and hardware.

use crate::tui::{self, App};
use anyhow::{anyhow, Result};
use argos_osint_core::brain::MemorySource;
use argos_osint_core::hardware;
use argos_osint_core::paths;
use argos_osint_core::provider::{self, SettingsFile};
use argos_osint_core::secrets::{AuthFile, ProviderSecret};
use argos_osint_core::store::Store;
use argos_osint_core::{osint, recon};
use clap::{Parser, Subcommand};
use std::io::{self, Write};
use std::sync::{atomic::AtomicBool, Arc};

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
    /// Uses hybrid recall (LanceDB vectors + Jaccard), or Jaccard when ARGOS_EMBED=0.
    Recall {
        query: String,
        #[arg(long, default_value_t = 8)]
        limit: usize,
    },
    /// List all saved memories as JSON, or manage the Brain vector index.
    Memories {
        #[command(subcommand)]
        command: Option<MemoriesCommand>,
    },
    /// List investigated claims with their evidence links.
    Insights {
        #[arg(long, default_value = "")]
        entity: String,
        #[arg(long, default_value = "")]
        topic: String,
    },
    /// Sign in or configure a provider account.
    Login,
    /// Forget saved provider credentials.
    Logout,
    /// Print the host profile.
    Hardware,
    /// List the model catalog for a role: recon, tool-picker, or synthesis.
    Models {
        #[arg(long, default_value = "synthesis")]
        role: String,
    },
    Recon {
        #[command(subcommand)]
        command: ReconCommand,
    },
    Osint {
        #[command(subcommand)]
        command: OsintCommand,
    },
    Defaults {
        #[command(subcommand)]
        command: DefaultsCommand,
    },
}

#[derive(Subcommand)]
enum MemoriesCommand {
    /// Rebuild the LanceDB vector index (memory_lancedb beside argos.db) from every
    /// memory and record the embedding fingerprint.
    Reindex,
}

#[derive(Subcommand)]
enum ReconCommand {
    List {
        #[arg(long, default_value = "")]
        search: String,
    },
    New {
        #[arg(long, default_value = "New investigation")]
        title: String,
    },
    Show {
        thread_id: String,
    },
    Ask {
        thread_id: String,
        question: String,
    },
    AskNew {
        question: String,
        #[arg(long, default_value = "New investigation")]
        title: String,
    },
    Rename {
        thread_id: String,
        title: String,
    },
    Delete {
        thread_id: String,
        #[arg(long)]
        with_insights: bool,
    },
    Retry {
        run_id: String,
    },
    Resume {
        run_id: String,
    },
    RetryInsights {
        answer_id: String,
    },
    Limits {
        #[arg(long)]
        max_rounds: Option<u8>,
        #[arg(long)]
        max_calls: Option<u8>,
        #[arg(long)]
        turn_seconds: Option<u16>,
        #[arg(long)]
        firecrawl_credits: Option<u32>,
        #[arg(long)]
        hunter_credits: Option<u32>,
        #[arg(long)]
        sociavault_credits: Option<u32>,
        #[arg(long)]
        firecrawl_trial: Option<u32>,
        #[arg(long)]
        hunter_trial: Option<u32>,
        #[arg(long)]
        sociavault_trial: Option<u32>,
        #[arg(long)]
        opening_hunter: Option<u8>,
        #[arg(long)]
        opening_sociavault: Option<u8>,
        /// monthly restores the recurring allowance; never keeps a fixed pool.
        #[arg(long)]
        credit_reset: Option<String>,
        #[arg(long)]
        firecrawl_search_cost: Option<u32>,
        #[arg(long)]
        firecrawl_scrape_cost: Option<u32>,
        #[arg(long)]
        hunter_call_cost: Option<u32>,
        #[arg(long)]
        sociavault_call_cost: Option<u32>,
        /// NewsAPI calls per turn (default 2).
        #[arg(long)]
        news_calls_per_turn: Option<u32>,
        /// CourtListener calls per turn (default 3).
        #[arg(long)]
        legal_calls_per_turn: Option<u32>,
        /// Hard ceiling for one turn, in seconds (default 900, range 120..1800).
        #[arg(long)]
        max_turn_seconds: Option<u16>,
    },
}

#[derive(Subcommand)]
enum OsintCommand {
    List,
    Describe {
        tool_id: String,
    },
    Run {
        tool_id: String,
        #[arg(long)]
        input: String,
    },
    History,
    Attach {
        call_id: String,
        thread_id: String,
    },
    Enable {
        tool_id: String,
    },
    Disable {
        tool_id: String,
    },
    UserAgent {
        value: String,
    },
}

#[derive(Subcommand)]
enum DefaultsCommand {
    Show,
    /// Set a role's provider and model. Role: recon, tool-picker, or synthesis.
    Set {
        role: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        model: String,
    },
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
        Some(Command::Memories {
            command: Some(MemoriesCommand::Reindex),
        }) => {
            let store = open_store()?;
            let report = store.reindex_memory_vectors()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "memories": report.memories,
                    "lance_dir": report.lance_dir.display().to_string(),
                    "fingerprint": report.fingerprint,
                }))?
            );
            Ok(())
        }
        Some(Command::Memories { command: None }) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&open_store()?.list_memories()?)?
            );
            Ok(())
        }
        Some(Command::Insights { entity, topic }) => {
            print_json(&open_store()?.search_insights(&entity, &topic)?)
        }
        Some(Command::Login) => login().await,
        Some(Command::Logout) => logout(),
        Some(Command::Hardware) => {
            let p = hardware::profile_cached(false);
            println!("{}", p.one_line());
            Ok(())
        }
        Some(Command::Models { role }) => models(&role).await,
        Some(Command::Recon { command }) => recon_command(command).await,
        Some(Command::Osint { command }) => osint_command(command).await,
        Some(Command::Defaults { command }) => defaults_command(command),
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
    println!("Provider account saved. Set Recon, Tool picker, and Synthesis models with `argos defaults set`.");
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

async fn models(role: &str) -> Result<()> {
    let auth = AuthFile::load()?;
    let settings = SettingsFile::load()?;
    let secret = provider::role_secret(&auth, &settings, role)?;
    let picker = provider::role_name(role) == Some("tool_picker");
    let kind = provider::effective_kind(&secret);
    if picker {
        println!(
            "{kind} / {} ({})",
            secret.model,
            provider::picker_transport(&secret.model)
        );
    } else {
        println!("{kind} / {}", secret.model);
    }
    let listed = match provider::list_models(&secret).await {
        Ok(models) => models,
        Err(err) => {
            eprintln!("catalog unavailable: {err}");
            Vec::new()
        }
    };
    for line in model_lines(picker, &kind, &listed) {
        println!("{line}");
    }
    Ok(())
}

/// Catalog lines for a role. The tool picker on OpenRouter always lists the decisions
/// models first, even when `GET /models` omits them.
fn model_lines(picker: bool, kind: &str, listed: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    if picker && kind == "openrouter" {
        for (id, label) in provider::DECISIONS_MODELS {
            lines.push(format!("{id}  {label}"));
        }
    }
    for model in listed {
        if picker && provider::DECISIONS_MODELS.iter().any(|(id, _)| id == model) {
            continue;
        }
        lines.push(model.clone());
    }
    lines
}

/// `defaults show` JSON: each role's provider and model, plus the picker transport.
fn defaults_json(auth: &AuthFile, settings: &SettingsFile) -> Result<serde_json::Value> {
    let role = |name: &str| -> Result<(String, String)> {
        let secret = provider::role_secret(auth, settings, name)?;
        Ok((provider::effective_kind(&secret), secret.model))
    };
    let (recon_provider, recon_model) = role("recon")?;
    let (picker_provider, picker_model) = role("tool-picker")?;
    let (synthesis_provider, synthesis_model) = role("synthesis")?;
    Ok(serde_json::json!({
        "recon": {"provider": recon_provider, "model": recon_model},
        "tool_picker": {
            "provider": picker_provider,
            "transport": provider::picker_transport(&picker_model),
            "model": picker_model,
        },
        "synthesis": {"provider": synthesis_provider, "model": synthesis_model},
    }))
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn defaults_command(command: DefaultsCommand) -> Result<()> {
    let mut settings = SettingsFile::load()?;
    match command {
        DefaultsCommand::Show => {
            let auth = AuthFile::load()?;
            print_json(&defaults_json(&auth, &settings)?)
        }
        DefaultsCommand::Set {
            role,
            provider,
            model,
        } => {
            anyhow::ensure!(!model.trim().is_empty(), "model is empty");
            let kind = provider::normalize_kind(&provider);
            anyhow::ensure!(
                matches!(
                    kind.as_str(),
                    "grok" | "openai" | "openai-chatgpt" | "openrouter" | "local"
                ),
                "unknown provider"
            );
            let target = settings
                .defaults
                .role_mut(&role)
                .ok_or_else(|| anyhow!("role must be recon, tool-picker, or synthesis"))?;
            target.provider = kind;
            target.model = model.trim().into();
            settings.save()?;
            print_json(&settings.defaults)
        }
    }
}

async fn osint_command(command: OsintCommand) -> Result<()> {
    match command {
        OsintCommand::List => print_json(&osint::registry()),
        OsintCommand::Describe { tool_id } => {
            let tool = osint::definition(&tool_id).ok_or_else(|| anyhow!("unknown tool"))?;
            print_json(
                &serde_json::json!({"tool":tool,"input_schema":tool.schema(),"example_input":tool.example_input()}),
            )
        }
        OsintCommand::History => print_json(&open_store()?.manual_calls()?),
        OsintCommand::Attach { call_id, thread_id } => print_json(
            &serde_json::json!({"attached":open_store()?.attach_call(&call_id,&thread_id)?}),
        ),
        OsintCommand::Enable { tool_id } => {
            open_store()?.set_tool_enabled(&tool_id, true)?;
            print_json(&serde_json::json!({"tool_id":tool_id,"enabled":true}))
        }
        OsintCommand::Disable { tool_id } => {
            open_store()?.set_tool_enabled(&tool_id, false)?;
            print_json(&serde_json::json!({"tool_id":tool_id,"enabled":false}))
        }
        OsintCommand::UserAgent { value } => {
            anyhow::ensure!(
                value.contains('@') || value.contains("http"),
                "include an identifying contact email or URL"
            );
            let mut settings = SettingsFile::load()?;
            settings.osint_user_agent = value.trim().to_string();
            settings.save()?;
            print_json(&serde_json::json!({"saved":true}))
        }
        OsintCommand::Run { tool_id, input } => {
            let value: serde_json::Value = serde_json::from_str(&input)?;
            let service =
                recon::Service::new(&paths::db_path(), AuthFile::load()?, SettingsFile::load()?)?;
            let (call_id, result) = service.manual(&tool_id, value).await?;
            print_json(&serde_json::json!({"call_id":call_id,"result":result}))
        }
    }
}

async fn recon_command(command: ReconCommand) -> Result<()> {
    match command {
        ReconCommand::List { search } => print_json(&open_store()?.list_threads(&search)?),
        ReconCommand::New { title } => print_json(&open_store()?.new_thread(&title)?),
        ReconCommand::Show { thread_id } => {
            let store = open_store()?;
            let thread = store
                .get_thread(&thread_id)?
                .ok_or_else(|| anyhow!("thread not found"))?;
            let messages = store.list_messages(&thread_id)?;
            // Runs carry the plan, including tool-picker picks and their probabilities.
            let runs: Vec<serde_json::Value> = store
                .runs_for_thread(&thread_id)?
                .into_iter()
                .map(|run| {
                    let plan = run
                        .plan_json
                        .as_deref()
                        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
                    serde_json::json!({
                        "id": run.id,
                        "state": run.state,
                        "recon_model": run.recon_model,
                        "tool_picker_model": run.tool_picker_model,
                        "synthesis_model": run.synthesis_model,
                        "plan": plan,
                    })
                })
                .collect();
            print_json(&serde_json::json!({"thread":thread,"messages":messages,"runs":runs}))
        }
        ReconCommand::Rename { thread_id, title } => print_json(
            &serde_json::json!({"renamed":open_store()?.rename_thread(&thread_id,&title)?}),
        ),
        ReconCommand::Delete {
            thread_id,
            with_insights,
        } => {
            let mut store = open_store()?;
            let removed = if with_insights {
                store.deletion_consequences(&thread_id)?
            } else {
                Vec::new()
            };
            let deleted = store.delete_thread(&thread_id, with_insights)?;
            print_json(&serde_json::json!({"deleted":deleted,"memories_removed":removed}))
        }
        ReconCommand::AskNew { question, title } => {
            let thread = open_store()?.new_thread(&title)?;
            ask_thread(&thread.id, &question).await
        }
        ReconCommand::Ask {
            thread_id,
            question,
        } => ask_thread(&thread_id, &question).await,
        ReconCommand::Retry { run_id } => {
            let store = open_store()?;
            let run = store
                .get_run(&run_id)?
                .ok_or_else(|| anyhow!("run not found"))?;
            let question = store
                .list_messages(&run.thread_id)?
                .into_iter()
                .find(|m| m.id == run.turn_id)
                .ok_or_else(|| anyhow!("original turn unavailable"))?
                .content;
            drop(store);
            ask_thread(&run.thread_id, &question).await
        }
        ReconCommand::Resume { run_id } => {
            open_store()?.recover_runs()?;
            let service =
                recon::Service::new(&paths::db_path(), AuthFile::load()?, SettingsFile::load()?)?;
            let run = service
                .resume(&run_id, Arc::new(AtomicBool::new(false)), |event| {
                    write_turn_event(&event, &mut std::io::stderr());
                })
                .await?;
            let store = open_store()?;
            print_json(
                &serde_json::json!({"run":run,"messages":store.list_messages(&run.thread_id)?,"calls":store.calls_for_run(&run.id)?}),
            )
        }
        ReconCommand::RetryInsights { answer_id } => {
            let service =
                recon::Service::new(&paths::db_path(), AuthFile::load()?, SettingsFile::load()?)?;
            service.retry_insights(&answer_id).await?;
            print_json(&serde_json::json!({"answer_id":answer_id,"insight_job":"completed"}))
        }
        ReconCommand::Limits {
            max_rounds,
            max_calls,
            turn_seconds,
            firecrawl_credits,
            hunter_credits,
            sociavault_credits,
            firecrawl_trial,
            hunter_trial,
            sociavault_trial,
            opening_hunter,
            opening_sociavault,
            credit_reset,
            firecrawl_search_cost,
            firecrawl_scrape_cost,
            hunter_call_cost,
            sociavault_call_cost,
            news_calls_per_turn,
            legal_calls_per_turn,
            max_turn_seconds,
        } => {
            let mut settings = SettingsFile::load()?;
            let mut changed = false;
            if let Some(value) = max_rounds {
                anyhow::ensure!((1..=8).contains(&value), "max-rounds must be 1..8");
                settings.recon_limits.max_rounds = value;
                changed = true;
            }
            if let Some(value) = max_calls {
                anyhow::ensure!((1..=24).contains(&value), "max-calls must be 1..24");
                settings.recon_limits.max_calls = value;
                changed = true;
            }
            if let Some(value) = turn_seconds {
                anyhow::ensure!(
                    (300..=900).contains(&value),
                    "turn-seconds must be 300..900"
                );
                settings.recon_limits.turn_seconds = value;
                changed = true;
            }
            if let Some(value) = max_turn_seconds {
                anyhow::ensure!(
                    (provider::MIN_MAX_TURN_SECONDS..=provider::MAX_MAX_TURN_SECONDS)
                        .contains(&value),
                    "max-turn-seconds must be 120..1800"
                );
                settings.recon_limits.max_turn_seconds = value;
                changed = true;
            }
            let assign = |slot: &mut u32, value: Option<u32>, changed: &mut bool| {
                if let Some(value) = value {
                    *slot = value;
                    *changed = true;
                }
            };
            assign(
                &mut settings.recon_limits.firecrawl_credits,
                firecrawl_credits,
                &mut changed,
            );
            assign(
                &mut settings.recon_limits.hunter_credits,
                hunter_credits,
                &mut changed,
            );
            assign(
                &mut settings.recon_limits.sociavault_credits,
                sociavault_credits,
                &mut changed,
            );
            assign(
                &mut settings.recon_limits.firecrawl_trial_credits,
                firecrawl_trial,
                &mut changed,
            );
            assign(
                &mut settings.recon_limits.hunter_trial_credits,
                hunter_trial,
                &mut changed,
            );
            assign(
                &mut settings.recon_limits.sociavault_trial_credits,
                sociavault_trial,
                &mut changed,
            );
            assign(
                &mut settings.recon_limits.firecrawl_search_cost,
                firecrawl_search_cost,
                &mut changed,
            );
            assign(
                &mut settings.recon_limits.firecrawl_scrape_cost,
                firecrawl_scrape_cost,
                &mut changed,
            );
            assign(
                &mut settings.recon_limits.hunter_call_cost,
                hunter_call_cost,
                &mut changed,
            );
            assign(
                &mut settings.recon_limits.sociavault_call_cost,
                sociavault_call_cost,
                &mut changed,
            );
            for (slot, value, name) in [
                (
                    &mut settings.recon_limits.news_calls_per_turn,
                    news_calls_per_turn,
                    "news-calls-per-turn",
                ),
                (
                    &mut settings.recon_limits.legal_calls_per_turn,
                    legal_calls_per_turn,
                    "legal-calls-per-turn",
                ),
            ] {
                if let Some(value) = value {
                    anyhow::ensure!(value <= 10, "{name} must be 0..10");
                    *slot = value;
                    changed = true;
                }
            }
            if let Some(value) = opening_hunter {
                anyhow::ensure!((0..=4).contains(&value), "opening-hunter must be 0..4");
                settings.recon_limits.opening_hunter_calls = value;
                changed = true;
            }
            if let Some(value) = opening_sociavault {
                anyhow::ensure!((0..=4).contains(&value), "opening-sociavault must be 0..4");
                settings.recon_limits.opening_sociavault_calls = value;
                changed = true;
            }
            if let Some(value) = credit_reset {
                anyhow::ensure!(
                    matches!(value.as_str(), "monthly" | "never"),
                    "credit-reset must be monthly or never"
                );
                settings.recon_limits.credit_reset = value;
                changed = true;
            }
            if changed {
                settings.save()?;
            }
            print_json(&settings.recon_limits)
        }
    }
}

async fn ask_thread(thread_id: &str, question: &str) -> Result<()> {
    let service = recon::Service::new(&paths::db_path(), AuthFile::load()?, SettingsFile::load()?)?;
    let run = service
        .ask(
            thread_id,
            question,
            Arc::new(AtomicBool::new(false)),
            |event| {
                write_turn_event(&event, &mut std::io::stderr());
            },
        )
        .await?;
    let store = open_store()?;
    print_json(
        &serde_json::json!({"run":run,"messages":store.list_messages(thread_id)?,"calls":store.calls_for_run(&run.id)?}),
    )
}

/// Synthesis tokens go to stderr as they arrive. Stdout stays the final JSON from `print_json`.
fn write_turn_event(event: &recon::TurnEvent, out: &mut impl std::io::Write) {
    match event {
        recon::TurnEvent::AnswerDelta(text) => {
            let _ = write!(out, "{text}");
            let _ = out.flush();
        }
        recon::TurnEvent::Stage(text)
        | recon::TurnEvent::AnswerNote(text)
        | recon::TurnEvent::Deadline(text) => {
            let _ = writeln!(out, "{text}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_show_includes_the_tool_picker_transport() {
        let mut settings = SettingsFile::default();
        settings.defaults.seed_tool_picker();
        settings.defaults.synthesis = provider::ModelAssignment {
            provider: "grok".into(),
            model: "grok-4.6".into(),
        };
        let shown = defaults_json(&AuthFile::default(), &settings).unwrap();
        assert_eq!(shown["tool_picker"]["provider"], "openrouter");
        assert_eq!(shown["tool_picker"]["model"], provider::TOOL_PICKER_MODEL);
        assert_eq!(shown["tool_picker"]["transport"], "decisions");
        assert_eq!(shown["synthesis"]["model"], "grok-4.6");
        settings.defaults.role_mut("tool-picker").unwrap().model = "x-ai/grok-4".into();
        let shown = defaults_json(&AuthFile::default(), &settings).unwrap();
        assert_eq!(shown["tool_picker"]["transport"], "chat");
        assert!(settings.defaults.role_mut("writer").is_none());
    }

    #[test]
    fn synthesis_deltas_go_to_the_event_stream_without_a_newline_between_tokens() {
        let mut out = Vec::new();
        write_turn_event(&recon::TurnEvent::Stage("synthesizing".into()), &mut out);
        write_turn_event(
            &recon::TurnEvent::Deadline("Deadline 6m 10s: 11 calls, ~52k chars evidence".into()),
            &mut out,
        );
        write_turn_event(&recon::TurnEvent::AnswerDelta("Hel".into()), &mut out);
        write_turn_event(&recon::TurnEvent::AnswerDelta("lo".into()), &mut out);
        write_turn_event(
            &recon::TurnEvent::AnswerNote("fixing citations…".into()),
            &mut out,
        );
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "synthesizing\nDeadline 6m 10s: 11 calls, ~52k chars evidence\nHellofixing citations…\n"
        );
    }

    #[test]
    fn tool_picker_catalog_lists_jev_even_when_the_api_omits_it() {
        let lines = model_lines(
            true,
            "openrouter",
            &["x-ai/grok-4".into(), provider::TOOL_PICKER_MODEL.into()],
        );
        assert_eq!(lines[0], "typesafe/jev-1.13  Jev 1.13 (decisions)");
        assert_eq!(lines.len(), 2);
        assert_eq!(
            model_lines(false, "openrouter", &["x-ai/grok-4".into()]),
            vec!["x-ai/grok-4".to_string()]
        );
        assert!(model_lines(true, "grok", &[]).is_empty());
    }
}
