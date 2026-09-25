//! `argos` command line. The default is the full-screen terminal UI.
//! `argos -p` runs one turn and prints it. `argos login` signs in Grok,
//! OpenAI, OpenRouter, or a local OpenAI-compatible server.

use std::io::{self, Read, Write};
use std::sync::{atomic::AtomicBool, Arc};

use anyhow::{anyhow, Result};
use argos_osint_core::agent::{self, TurnEvent, TurnInput};
use argos_osint_core::gmail::GmailConfig;
use argos_osint_core::hardware;
use argos_osint_core::mcp::{self, take_frame};
use argos_osint_core::paths;
use argos_osint_core::provider::{self, SettingsFile};
use argos_osint_core::secrets::{AuthFile, ProviderSecret};
use argos_osint_core::store::Store;
use clap::{Parser, Subcommand};
use tokio::sync::mpsc::unbounded_channel;

use crate::tui::{self, App};

#[derive(Parser)]
#[command(
    name = "argos",
    version,
    about = "Argos OSINT — terminal research desk"
)]
struct Cli {
    /// Run one turn and print the answer instead of opening the TUI.
    #[arg(short = 'p', long)]
    prompt: Option<String>,
    /// Text model for this process. Saved when opening the TUI.
    #[arg(short = 'm', long)]
    model: Option<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Sign in a text or voice provider from the terminal.
    Login,
    /// Forget text and voice credentials. Pass --gmail to forget the mailbox too.
    Logout {
        #[arg(long)]
        gmail: bool,
    },
    /// Print the host profile.
    Hardware,
    /// List markdown reports.
    Reports,
    /// List models for the active text provider. Grok starts with its built-in catalog.
    Models,
    /// Speak MCP over stdio. Only `gmail` is implemented.
    Mcp { service: String },
}

pub async fn dispatch() -> Result<()> {
    let cli = Cli::parse();
    if let Some(command) = cli.command {
        return match command {
            Command::Login => login().await,
            Command::Logout { gmail } => logout(gmail),
            Command::Hardware => print_hardware(),
            Command::Reports => print_reports(),
            Command::Models => print_models().await,
            Command::Mcp { service } => mcp(service),
        };
    }
    if let Some(prompt) = cli.prompt {
        return headless(prompt, cli.model).await;
    }
    let mut app = App::boot()?;
    if let Some(model) = cli.model {
        app.select_model(&model);
    }
    tui::run(app).await
}

async fn print_models() -> Result<()> {
    let settings = SettingsFile::load().unwrap_or_default();
    let auth = AuthFile::load().unwrap_or_default();
    let secret = provider::active_text_secret(&auth, &settings.model);
    println!("{}  {}", provider::effective_kind(&secret), secret.base_url);
    if provider::effective_kind(&secret) == "grok" {
        for model in provider::grok_models() {
            let mark = if model.id == secret.model { "*" } else { " " };
            println!("{mark} {:<12} {}", model.id, model.name);
        }
    }
    match provider::list_models(&secret).await {
        Ok(names) => {
            if !names.is_empty() {
                println!("endpoint:");
                for name in names {
                    let mark = if name == secret.model { "*" } else { " " };
                    println!("{mark} {name}");
                }
            }
        }
        Err(err) => println!("endpoint list unavailable: {err}"),
    }
    Ok(())
}

fn print_hardware() -> Result<()> {
    let profile = hardware::profile_cached(false);
    println!("{}", profile.one_line());
    println!("backend: {}", profile.backend);
    println!("logical cores: {}", profile.logical_cores);
    if let Some(cores) = profile.gpu_cores {
        println!("gpu cores: {cores}");
    }
    Ok(())
}

fn print_reports() -> Result<()> {
    paths::ensure_home()?;
    let store = Store::open(&paths::db_path())?;
    let reports = store.list_reports()?;
    if reports.is_empty() {
        println!("No reports yet.");
        return Ok(());
    }
    for report in reports {
        println!("{}\t{}", report.created_at, report.path);
    }
    Ok(())
}

fn logout(gmail: bool) -> Result<()> {
    let mut auth = AuthFile::load()?;
    auth.text = None;
    auth.voice = None;
    if gmail {
        auth.gmail = None;
    }
    auth.save()?;
    println!("Signed out text and voice providers.");
    if gmail {
        println!("Forgot the Gmail app password.");
    }
    Ok(())
}

async fn login() -> Result<()> {
    println!("Argos provider login");
    println!("Chat and voice use the OpenAI-compatible API. The provider only changes the host and the key.");
    println!(
        "Credentials are stored in {} with owner-only permissions.",
        paths::auth_path().display()
    );
    println!();
    for preset in provider::presets() {
        let auth = match preset.env_key {
            Some(name) if preset.key_required => format!("key or {name}"),
            Some(name) => format!("optional key or {name}"),
            None => "optional key".into(),
        };
        println!("  {:<12} {}  ({auth})", preset.id, preset.base_url);
    }
    println!();
    let modality = ask("Modality [text/voice]", "text")?;
    let choice = ask("Provider [grok/openai/openrouter/local]", "grok")?;
    let Some(preset) = provider::preset(&choice) else {
        return Err(anyhow!(
            "unknown provider {choice}. Choose grok, openai, openrouter, or local."
        ));
    };
    let base_url = ask("Base URL", preset.base_url)?;
    let default_model = if modality == "voice" {
        preset.voice_model
    } else {
        preset.text_model
    };
    let model = ask("Model", default_model)?;
    let api_key = prompt_provider_key(preset)?;
    let secret = ProviderSecret {
        kind: preset.id.to_string(),
        base_url,
        model: model.clone(),
        api_key,
        stt_model: if modality == "voice" {
            Some(model)
        } else {
            Some(preset.voice_model.to_string())
        },
        device: None,
    };
    let mut auth = AuthFile::load()?;
    if modality == "voice" {
        auth.voice = Some(secret);
    } else {
        auth.text = Some(secret);
    }
    auth.save()?;
    println!("Saved the {modality} slot on {}.", preset.label);
    Ok(())
}

fn prompt_provider_key(preset: &provider::ProviderPreset) -> Result<Option<String>> {
    if let Some(name) = preset.env_key {
        if std::env::var(name)
            .ok()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        {
            let use_env = ask(&format!("Use {name} from the environment? [Y/n]"), "Y")?;
            if !matches!(use_env.to_lowercase().as_str(), "n" | "no") {
                println!("{name} will be read at request time and is not copied into auth.json.");
                return Ok(None);
            }
        }
    }
    let prompt = if preset.key_required {
        "API key: "
    } else {
        "API key (empty if this server does not need one): "
    };
    let key = rpassword::prompt_password(prompt)?;
    let key = key.trim().to_string();
    if key.is_empty() {
        if preset.key_required {
            return Err(anyhow!(
                "{} needs an API key{}",
                preset.label,
                preset
                    .env_key
                    .map(|name| format!(" or {name}"))
                    .unwrap_or_default()
            ));
        }
        return Ok(None);
    }
    Ok(Some(key))
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
    let line = line.trim().to_string();
    if line.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(line)
    }
}

fn writer_model_id(settings: &SettingsFile, override_model: Option<&str>) -> String {
    if let Some(model) = override_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        return model.to_string();
    }
    if !settings.writer_model.trim().is_empty() {
        return settings.writer_model.trim().to_string();
    }
    settings.model.clone()
}

fn tool_model_id(settings: &SettingsFile, override_model: Option<&str>) -> String {
    if !settings.tool_model.trim().is_empty() {
        return settings.tool_model.trim().to_string();
    }
    writer_model_id(settings, override_model)
}

async fn headless(prompt: String, model: Option<String>) -> Result<()> {
    paths::ensure_home()?;
    let store = Store::open(&paths::db_path())?;
    store.ensure_session("desk", "Desk", "desk")?;
    let settings = SettingsFile::load().unwrap_or_default();
    let auth = AuthFile::load().unwrap_or_default();
    let profile = hardware::profile_cached(false);
    let input = TurnInput {
        session_id: "desk".into(),
        user_text: prompt,
        history: Vec::new(),
        memories: store.list_memories()?,
        view_name: "Desk".into(),
        view_context: "Headless turn. No canvas is open.".into(),
        hardware_line: profile.one_line(),
        modality: settings.modality.clone(),
        provider: Some(provider::active_text_secret(
            &auth,
            &writer_model_id(&settings, model.as_deref()),
        )),
        tool_provider: Some(provider::active_text_secret(
            &auth,
            &tool_model_id(&settings, model.as_deref()),
        )),
        plan: settings.source_plan(),
        report_dir: crate::tui::report_dir(&settings),
        case_id: None,
        gmail: auth.gmail.as_ref().map(GmailConfig::from),
        prior_reports: String::new(),
        evidence_only: false,
        from_memory: false,
    };
    let (tx, mut rx) = unbounded_channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker = tokio::spawn(async move {
        agent::run_turn(input, tx, cancel).await;
    });
    let mut final_text = String::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            TurnEvent::Delta(text) => {
                print!("{text}");
                let _ = io::stdout().flush();
            }
            TurnEvent::Note(text) | TurnEvent::Status(text) => eprintln!("{text}"),
            TurnEvent::Report(meta) => {
                eprintln!("report {}", meta.path);
                let _ = store.add_report(&meta);
            }
            TurnEvent::Memory(memory) => {
                let _ = store.add_memory(&memory.text);
            }
            TurnEvent::Done(text) => final_text = text,
            TurnEvent::Failed(err) => return Err(anyhow!(err)),
        }
    }
    if !final_text.is_empty() {
        println!("\n{final_text}");
    }
    let _ = worker.await;
    Ok(())
}

fn mcp(service: String) -> Result<()> {
    if !service.eq_ignore_ascii_case("gmail") {
        return Err(anyhow!("only the gmail MCP service is available"));
    }
    let auth = AuthFile::load()?;
    let cfg = auth.gmail.as_ref().map(GmailConfig::from);
    eprintln!("argos gmail mcp");
    let mut incoming = String::new();
    let mut stdin = io::stdin();
    let mut buf = [0u8; 4096];
    loop {
        let n = stdin.read(&mut buf)?;
        if n == 0 {
            break;
        }
        incoming.push_str(&String::from_utf8_lossy(&buf[..n]));
        while let Some((message, used)) = take_frame(&incoming) {
            incoming = incoming[used..].to_string();
            if let Some(response) = mcp::handle_message(&message, cfg.as_ref()) {
                println!("{response}");
                let _ = io::stdout().flush();
            }
        }
    }
    Ok(())
}
