//! `argos` command line. The default is the full-screen terminal UI.
//! `argos -p` runs one turn and prints it. `argos login` is the terminal
//! provider flow (API key, local endpoint, or RFC 8628 device code).

use std::io::{self, Read, Write};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;

use anyhow::{anyhow, Result};
use argos_osint_core::agent::{self, TurnEvent, TurnInput};
use argos_osint_core::gmail::GmailConfig;
use argos_osint_core::hardware;
use argos_osint_core::mcp::{self, take_frame};
use argos_osint_core::paths;
use argos_osint_core::provider::{self, Poll, SettingsFile};
use argos_osint_core::secrets::{AuthFile, DeviceEndpoints, ProviderSecret};
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
            Command::Mcp { service } => mcp(service),
        };
    }
    if let Some(prompt) = cli.prompt {
        return headless(prompt).await;
    }
    let app = App::boot()?;
    tui::run(app).await
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
    println!(
        "Credentials are stored in {} with owner-only permissions.",
        paths::auth_path().display()
    );
    let modality = ask("Modality [text/voice]", "text")?;
    let kind = ask("Kind [local/api/device]", "local")?;
    let default_base = if kind == "local" {
        "http://127.0.0.1:11434/v1"
    } else {
        "https://api.x.ai/v1"
    };
    let base_url = ask("Base URL", default_base)?;
    let model = ask(
        "Model",
        if modality == "voice" {
            "whisper-1"
        } else {
            "grok-4"
        },
    )?;
    let mut secret = ProviderSecret {
        kind: kind.clone(),
        base_url,
        model: model.clone(),
        api_key: None,
        stt_model: if modality == "voice" {
            Some(model)
        } else {
            None
        },
        device: None,
    };
    match kind.as_str() {
        "device" => {
            let endpoints = DeviceEndpoints {
                client_id: ask("OAuth client id", "")?,
                device_auth_url: ask("Device authorization URL", "")?,
                token_url: ask("Token URL", "")?,
                scope: ask("Scope", "openid profile email")?,
            };
            if endpoints.client_id.is_empty() {
                return Err(anyhow!("device login needs a client id"));
            }
            let grant = provider::start_device(&endpoints).await?;
            println!();
            println!("Open: {}", grant.verification_uri);
            if let Some(url) = &grant.verification_uri_complete {
                println!("Or:   {url}");
            }
            println!("Code: {}", grant.user_code);
            println!("Waiting for approval. Ctrl+C cancels.");
            let mut interval = grant.interval.max(2);
            let mut waited = 0u64;
            loop {
                if waited >= grant.expires_in {
                    return Err(anyhow!("device code expired"));
                }
                tokio::time::sleep(Duration::from_secs(interval)).await;
                waited = waited.saturating_add(interval);
                match provider::poll_device(&endpoints, &grant.device_code).await? {
                    Poll::Pending => print!("."),
                    Poll::SlowDown => interval = interval.saturating_add(5),
                    Poll::Token(token) => {
                        println!();
                        secret.api_key = Some(token);
                        secret.device = Some(endpoints);
                        break;
                    }
                    Poll::Denied(err) => return Err(anyhow!(err)),
                }
                let _ = io::stdout().flush();
            }
        }
        "api" => {
            let key = rpassword::prompt_password("API key: ")?;
            if key.trim().is_empty() {
                return Err(anyhow!("empty API key"));
            }
            secret.api_key = Some(key.trim().to_string());
        }
        _ => {
            let key = ask("API key (empty if the local server does not need one)", "")?;
            if !key.is_empty() {
                secret.api_key = Some(key);
            }
        }
    }
    let mut auth = AuthFile::load()?;
    if modality == "voice" {
        auth.voice = Some(secret);
    } else {
        auth.text = Some(secret);
    }
    auth.save()?;
    println!("Saved the {modality} provider.");
    Ok(())
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

async fn headless(prompt: String) -> Result<()> {
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
        provider: auth.text,
        searx_url: {
            let url = settings.searx_url.clone();
            Some(url).filter(|s| !s.is_empty())
        },
        report_dir: crate::tui::report_dir(&settings),
        case_id: None,
        gmail: auth.gmail.as_ref().map(GmailConfig::from),
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
