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
}

fn arguments() -> Result<(Options, Vec<String>), clap::Error> {
    let mut native = vec!["fleetworks-desktop".to_owned()];
    let mut tooling = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        if matches!(
            argument.as_str(),
            "--api-origin" | "--api-token-file" | "--workspace"
        ) {
            native.push(argument);
            if let Some(value) = args.next() {
                native.push(value);
            }
        } else if argument.starts_with("--api-origin=")
            || argument.starts_with("--api-token-file=")
            || argument.starts_with("--workspace=")
            || argument == "--provision-gateway-credential"
            || matches!(argument.as_str(), "--help" | "-h" | "--version" | "-V")
        {
            native.push(argument);
        } else {
            tooling.push(argument);
        }
    }
    Ok((Options::try_parse_from(native)?, tooling))
}

/// Start the protected native shell without starting or configuring a backend.
pub fn run() -> anyhow::Result<()> {
    let (options, tooling) = match arguments() {
        Ok(parsed) => parsed,
        Err(error) => error.exit(),
    };
    if options.provision_gateway_credential {
        let container = fleetworks_workspace::native::NativeContainer::open_settings(
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
            state.connection_profiles.launch_override =
                options.api_origin.is_some() || options.api_token_file.is_some();
            if let Some(origin) = &options.api_origin {
                state.connection.origin = TextEdit::new(origin);
            }
            if let Some(path) = &options.api_token_file {
                state.connection.token_file = TextEdit::new(path.to_string_lossy());
            }
            state.initialize_workspaces(options.workspace.clone());
            state
        })
        .args(tooling)
        .chrome(Chrome {
            full_size_content: true,
            traffic_lights: Some((16.0, 17.0)),
            drag_regions: true,
        })
        .on_focus(|state, focused, _| {
            if focused {
                state.accessibility.invalidate();
            }
        })
        .on_close(|state, _| state.cancel_requests());
    services
        .install_ui(app.handle())
        .map_err(anyhow::Error::msg)?;
    app.run();
    runtime.shutdown_timeout(Duration::from_secs(2));
    Ok(())
}
