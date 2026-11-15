//! Native launch arguments and the one application-owned asynchronous runtime.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use rune::App;
use rune::Chrome;
use rune::TextEdit;

use crate::app;
use crate::connection::GatewayEffects;
use crate::local_backend::{resolve_companion_binary, validate_companion_executable};
use crate::State;

#[derive(Debug, Parser)]
#[command(
    name = "fleetworks-desktop",
    version,
    about = "Connect to an independently running Fleetworks gateway"
)]
struct Options {
    #[arg(long)]
    api_origin: Option<String>,
    #[arg(long)]
    api_token_file: Option<PathBuf>,
    #[arg(long)]
    workspace: Option<PathBuf>,
    #[arg(long, requires_all = ["api_origin", "api_token_file"])]
    provision_gateway_credential: bool,
    #[arg(long)]
    local_backend_bin: Option<PathBuf>,
    #[arg(long)]
    diagnostic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiagnosticReport {
    platform: &'static str,
    mode_hint: &'static str,
    companion_status: String,
    api_origin_configured: bool,
    api_token_file_configured: bool,
}

fn parse_options(
    args: impl IntoIterator<Item = String>,
) -> Result<(Options, Vec<String>), clap::Error> {
    let mut native = vec!["fleetworks-desktop".to_owned()];
    let mut tooling = Vec::new();
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        if matches!(
            argument.as_str(),
            "--api-origin" | "--api-token-file" | "--workspace" | "--local-backend-bin"
        ) {
            native.push(argument);
            if let Some(value) = args.next() {
                native.push(value);
            }
        } else if argument.starts_with("--api-origin=")
            || argument.starts_with("--api-token-file=")
            || argument.starts_with("--workspace=")
            || argument.starts_with("--local-backend-bin=")
            || argument == "--provision-gateway-credential"
            || argument == "--diagnostic"
            || matches!(argument.as_str(), "--help" | "-h" | "--version" | "-V")
        {
            native.push(argument);
        } else {
            tooling.push(argument);
        }
    }
    Ok((Options::try_parse_from(native)?, tooling))
}

fn arguments() -> Result<(Options, Vec<String>), clap::Error> {
    parse_options(std::env::args().skip(1))
}

fn init_diagnostic_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn diagnostic_preflight(options: &Options) -> anyhow::Result<DiagnosticReport> {
    anyhow::ensure!(
        options.workspace.is_none(),
        "--workspace is retired; this installation uses workstation identity"
    );
    let companion_status = if let Some(path) = &options.local_backend_bin {
        let valid = validate_companion_executable(path)?;
        format!("ready ({})", valid.display())
    } else {
        match resolve_companion_binary(None) {
            Ok(path) => format!("ready ({})", path.display()),
            Err(error) => format!("unavailable ({error})"),
        }
    };
    let mode_hint = if options.api_origin.is_some() || options.api_token_file.is_some() {
        "remote-launch-override"
    } else if options.local_backend_bin.is_some() || companion_status.starts_with("ready (") {
        "local-or-remote"
    } else {
        "remote-gateway"
    };
    Ok(DiagnosticReport {
        platform: std::env::consts::OS,
        mode_hint,
        companion_status,
        api_origin_configured: options.api_origin.is_some(),
        api_token_file_configured: options.api_token_file.is_some(),
    })
}

/// Start the protected native shell without starting or configuring a backend.
pub fn run() -> anyhow::Result<()> {
    let (options, tooling) = match arguments() {
        Ok(parsed) => parsed,
        Err(error) => error.exit(),
    };
    if options.diagnostic {
        init_diagnostic_tracing();
        let report = diagnostic_preflight(&options)?;
        eprintln!(
            "fleetworks-desktop diagnostic startup: platform={} mode={} api_origin_configured={} api_token_file_configured={} companion={}",
            report.platform,
            report.mode_hint,
            report.api_origin_configured,
            report.api_token_file_configured,
            report.companion_status
        );
    }
    if options.provision_gateway_credential {
        let container = fleetworks_workstation::native::NativeContainer::open_settings(
            &crate::platform::home()?,
        )?;
        container.provision_credential_binding(
            options.api_origin.as_deref().expect("clap requires origin"),
            options
                .api_token_file
                .as_deref()
                .expect("clap requires credential file"),
        )?;
        println!("Gateway credential reference provisioned. No token bytes were stored.");
        return Ok(());
    }
    anyhow::ensure!(
        options.workspace.is_none(),
        "--workspace is retired; this installation uses workstation identity"
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let services = Arc::new(GatewayEffects::new(runtime.handle().clone()));
    let init_services = services.clone();
    let app = App::new(app::spec())
        .init(move || {
            let mut state = State::with_services(init_services.clone());
            state.connection_profiles.operator_token_override = options.api_token_file.is_some();
            state.connection_profiles.launch_override =
                options.api_origin.is_some() || options.api_token_file.is_some();
            if let Some(origin) = &options.api_origin {
                state.connection.origin = TextEdit::new(origin);
            }
            if let Some(path) = &options.api_token_file {
                state.connection.token_file = TextEdit::new(path.to_string_lossy());
            }
            state.local_backend.cli_binary_override = options.local_backend_bin.clone();
            state.local_backend.refresh_binary_status();
            state.initialize_workspaces(options.workspace.clone());
            state
        })
        .args(tooling)
        .chrome(if cfg!(target_os = "macos") {
            Chrome {
                full_size_content: true,
                traffic_lights: Some((16.0, 17.0)),
                drag_regions: true,
            }
        } else {
            Chrome {
                full_size_content: false,
                traffic_lights: None,
                drag_regions: false,
            }
        })
        .on_focus(|state, focused, _| {
            state.pairing_focus(focused);
            if focused {
                state.accessibility.invalidate();
            }
        })
        .on_close(|state, _| state.close_window_cleanup())
        .on_quit(|state, _| state.quit_cleanup());
    services
        .install_ui(app.handle())
        .map_err(anyhow::Error::msg)?;
    let result = app.try_run();
    runtime.shutdown_timeout(Duration::from_secs(2));
    result.map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn parses_diagnostic_and_forwards_unknown_tooling_args() {
        let (options, tooling) = parse_options([
            "--diagnostic".to_owned(),
            "--api-origin=https://gateway.example.test".to_owned(),
            "--custom-rune-flag".to_owned(),
        ])
        .expect("parse diagnostic options");
        assert!(options.diagnostic);
        assert_eq!(
            options.api_origin.as_deref(),
            Some("https://gateway.example.test")
        );
        assert_eq!(tooling, vec!["--custom-rune-flag"]);
    }

    #[test]
    fn diagnostic_preflight_validates_explicit_companion_and_rejects_retired_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("missing-argos-api-host");
        let (missing_opts, _) = parse_options([
            "--diagnostic".to_owned(),
            "--local-backend-bin".to_owned(),
            missing.to_string_lossy().into_owned(),
        ])
        .expect("parse options");
        let err = diagnostic_preflight(&missing_opts).expect_err("missing binary must fail closed");
        assert!(
            err.to_string().contains("unavailable"),
            "unexpected error: {err}"
        );

        let (retired_opts, _) = parse_options([
            "--diagnostic".to_owned(),
            "--workspace".to_owned(),
            dir.path().to_string_lossy().into_owned(),
        ])
        .expect("parse options");
        let err = diagnostic_preflight(&retired_opts).expect_err("retired workspace must fail");
        assert!(err.to_string().contains("--workspace is retired"));

        let bin = dir.path().join("argos-api-host");
        std::fs::write(&bin, b"#!/bin/sh\nexit 0\n").expect("write mock companion");
        let mut perms = std::fs::metadata(&bin).expect("metadata").permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&bin, perms).expect("chmod");

        let (valid_opts, _) = parse_options([
            "--diagnostic".to_owned(),
            "--local-backend-bin".to_owned(),
            bin.to_string_lossy().into_owned(),
        ])
        .expect("parse valid options");
        let report = diagnostic_preflight(&valid_opts).expect("valid diagnostic preflight");
        assert!(report.companion_status.starts_with("ready ("));
        assert_eq!(report.mode_hint, "local-or-remote");
    }
}
