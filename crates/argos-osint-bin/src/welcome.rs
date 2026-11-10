//! Service setup before any application opens storage.
pub(crate) mod classifier;
pub(crate) mod seed;
mod terminal;
use std::io::{self, BufRead, IsTerminal, Write};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub args: Vec<String>,
    pub storage_init: Option<Vec<String>>,
}

trait Prompts {
    fn message(&mut self, message: &str) -> io::Result<()>;
    fn ask(
        &mut self,
        label: &str,
        default: &str,
        valid: &dyn Fn(&str) -> bool,
    ) -> io::Result<Option<String>>;
}

struct Lines<'a, R, W> {
    input: &'a mut R,
    output: &'a mut W,
}
impl<R: BufRead, W: Write> Prompts for Lines<'_, R, W> {
    fn message(&mut self, message: &str) -> io::Result<()> {
        writeln!(self.output, "{message}")
    }
    fn ask(
        &mut self,
        label: &str,
        default: &str,
        valid: &dyn Fn(&str) -> bool,
    ) -> io::Result<Option<String>> {
        ask(self.input, self.output, label, default, valid)
    }
}

pub(crate) fn run_with_notice(notice: &str) -> anyhow::Result<Option<Selection>> {
    let mut existing = None;
    #[cfg(feature = "surreal")]
    {
        use argos_osint_core::storage::home;
        let layout = home::Layout::new(argos_osint_core::paths::ensure_home()?);
        if home::load_config_file(&layout)?.is_some() || std::env::var_os(home::MODE_ENV).is_some()
        {
            existing = home::load_config(&layout)?.map(|config| match config.mode {
                home::Mode::Embedded => "embedded",
                home::Mode::Server => "server",
            });
        }
    }
    #[cfg(feature = "surreal")]
    let needs_seed = {
        let output = std::process::Command::new(std::env::current_exe()?)
            .args(["storage", "actor-seed-status"])
            .output()?;
        if output.status.success() {
            serde_json::from_slice::<argos_osint_core::storage::actor_catalog::Status>(
                &output.stdout,
            )?
            .needs_seed
        } else {
            // An unavailable/locked database is not an empty database. Keep
            // the normal launcher usable and surface the probe failure.
            eprintln!(
                "Actor seed status unavailable: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            false
        }
    };
    #[cfg(not(feature = "surreal"))]
    let needs_seed = false;
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return terminal::run(existing, notice, needs_seed);
    }
    eprintln!("{notice}");
    Ok(configure_launcher(
        &mut Lines {
            input: &mut io::stdin().lock(),
            output: &mut io::stdout().lock(),
        },
        existing,
        needs_seed,
    )?)
}

fn ask(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
    default: &str,
    valid: impl Fn(&str) -> bool,
) -> io::Result<Option<String>> {
    loop {
        write!(
            output,
            "{label}{}: ",
            if default.is_empty() {
                String::new()
            } else {
                format!(" [{default}]")
            }
        )?;
        output.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            writeln!(output)?;
            return Ok(None);
        }
        let value = line.trim();
        if matches!(value.to_ascii_lowercase().as_str(), "q" | "quit" | "exit") {
            return Ok(None);
        }
        let value = if value.is_empty() { default } else { value };
        if valid(value) {
            return Ok(Some(value.to_owned()));
        }
        writeln!(
            output,
            "Invalid value. Enter one of the listed choices or a valid port/loopback URL; q quits."
        )?;
    }
}

fn port(value: &str) -> bool {
    value.parse::<u16>().is_ok_and(|port| port > 0)
}

#[cfg(test)]
fn select(
    input: &mut impl BufRead,
    output: &mut impl Write,
    existing: Option<&str>,
) -> io::Result<Option<Selection>> {
    configure(&mut Lines { input, output }, existing)
}

#[cfg(test)]
fn configure(ui: &mut impl Prompts, existing: Option<&str>) -> io::Result<Option<Selection>> {
    configure_launcher(ui, existing, false)
}

fn dataset_path(value: &str) -> Option<&str> {
    let value = value.trim();
    let value = if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        value.get(1..value.len().saturating_sub(1)).unwrap_or("")
    } else {
        value
    };
    (!value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control))
        .then_some(value)
}

fn append_dataset_file(args: &mut Vec<String>, selection: &str) {
    if let Some(path) = dataset_path(selection).filter(|path| *path != "auto") {
        args.extend(["--file".into(), path.into()]);
    }
}

fn configure_launcher(
    ui: &mut impl Prompts,
    existing: Option<&str>,
    needs_seed: bool,
) -> io::Result<Option<Selection>> {
    macro_rules! prompt {
        ($label:expr, $default:expr, $valid:expr) => {
            match ui.ask($label, $default, &$valid)? {
                Some(value) => value,
                None => return Ok(None),
            }
        };
    }
    let banner = crate::tui::wordmark::venture_rows()
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    ui.message(&format!(
        "{banner}\nWelcome to Argos Venture! Which service would you like to run?\n"
    ))?;
    let mut choices = String::from("  1. TUI\n  2. API Server\n  3. Web\n");
    if needs_seed {
        choices.push_str("  4. Seed DB\n");
    }
    choices.push_str("  5. Check Atlas classifier\n");
    if cfg!(feature = "classifier") {
        choices.push_str("  6. Train legacy classifier\n");
    }
    choices.push_str("  q. Quit\n");
    ui.message(&choices)?;
    let service = prompt!("Choose a service", "", |value: &str| {
        let value = value.to_ascii_lowercase();
        matches!(
            value.as_str(),
            "1" | "tui"
                | "2"
                | "api"
                | "api server"
                | "server"
                | "3"
                | "web"
                | "5"
                | "check atlas classifier"
        ) || (needs_seed && matches!(value.as_str(), "4" | "seed" | "seed db"))
            || (cfg!(feature = "classifier")
                && matches!(
                    value.as_str(),
                    "6" | "train" | "train classifier" | "train legacy classifier"
                ))
    });
    if matches!(
        service.to_ascii_lowercase().as_str(),
        "5" | "check atlas classifier"
    ) {
        return Ok(Some(Selection {
            args: vec!["argos".into(), "classifier".into(), "status".into()],
            storage_init: None,
        }));
    }
    if matches!(
        service.to_ascii_lowercase().as_str(),
        "6" | "train" | "train classifier" | "train legacy classifier"
    ) {
        ui.message(
            "Train and activate a synthetic Natural baseline for legacy classifier operations.",
        )?;
        let file = prompt!(
            "Classifier CSV path (auto for configured/default)",
            "auto",
            |value: &str| dataset_path(value).is_some()
        );
        let mut args = vec![
            "argos".into(),
            "classifier".into(),
            "train".into(),
            "--activate-synthetic-baseline".into(),
        ];
        append_dataset_file(&mut args, &file);
        return Ok(Some(Selection {
            args,
            storage_init: None,
        }));
    }
    if matches!(
        service.to_ascii_lowercase().as_str(),
        "4" | "seed" | "seed db"
    ) {
        let file = prompt!(
            "Actor CSV path (auto for configured/default)",
            "auto",
            |value: &str| dataset_path(value).is_some()
        );
        let mut args = vec!["argos".into(), "storage".into(), "seed-actors".into()];
        append_dataset_file(&mut args, &file);
        return Ok(Some(Selection {
            args,
            storage_init: existing.is_none().then(|| {
                vec![
                    "storage".into(),
                    "init".into(),
                    "--mode".into(),
                    "server".into(),
                ]
            }),
        }));
    }
    let target = match service.to_ascii_lowercase().as_str() {
        "1" | "tui" => "--tui",
        "2" | "api" | "api server" | "server" => "--server",
        _ => "--web",
    };
    let mut selection = Selection {
        args: vec!["argos".into(), "stack".into(), "up".into(), target.into()],
        storage_init: None,
    };
    let mut adopted = false;
    let mut web_port = String::new();
    if target == "--web" {
        web_port = prompt!("Web port (1-65535)", "3000", port);
        selection.args.extend(["--port".into(), web_port.clone()]);
        let profile = prompt!("Web profile [dev/installed]", "dev", |v| matches!(
            v,
            "dev" | "installed"
        ));
        selection.args.extend(["--profile".into(), profile]);
        let backend = prompt!("API [start/connect]", "start", |v| matches!(
            v,
            "start" | "connect"
        ));
        if backend == "connect" {
            let url = prompt!("Existing API loopback URL", "http://127.0.0.1:8080", |v| {
                crate::stack::loopback_url(v).is_ok()
            });
            selection.args.extend(["--api-url".into(), url]);
            adopted = true;
            ui.message("Storage is managed by the existing API. Its Origin allowlist must permit this web port. The terminal display for an adopted API requires this home's server storage.")?;
        }
    }
    if target != "--tui" && !adopted {
        let api_port = prompt!(
            "API port (1-65535; different from web port)",
            "8080",
            |v| port(v)
                && (web_port.is_empty() || v.parse::<u16>().ok() != web_port.parse::<u16>().ok())
        );
        selection.args.extend(["--api-port".into(), api_port]);
    }
    if !adopted {
        if let Some(mode) = existing {
            ui.message(&format!("Current storage mode: {mode}. Switching an initialized home requires the explicit storage workflow; choose another ARGOS_HOME for a separate store."))?;
            let choices = if mode == "embedded" {
                "keep"
            } else {
                "keep/server"
            };
            prompt!(&format!("Storage mode [{choices}]"), "keep", |v| {
                v == "keep" || (v == "server" && mode == "server")
            });
        } else {
            let mode = prompt!("Storage mode [server]", "server", |v| v == "server");
            let mut args = vec![
                "storage".into(),
                "init".into(),
                "--mode".into(),
                mode.clone(),
            ];
            if mode == "server" {
                ui.message("SurrealDB server storage requires the pinned server binary (argos storage install). This port is separate from the HTTP API port.")?;
                let db_port = prompt!("SurrealDB port [auto or 1-65535]", "auto", |v| v == "auto"
                    || (port(v)
                        && !selection.args.windows(2).any(|a| matches!(
                            a[0].as_str(),
                            "--port" | "--api-port"
                        ) && a[1].parse::<u16>().ok()
                            == v.parse::<u16>().ok())));
                if db_port != "auto" {
                    args.extend(["--port".into(), db_port]);
                }
            }
            selection.storage_init = Some(args);
        }
    }
    ui.message(&format!("Launch: {}", selection.args.join(" ")))?;
    if let Some(args) = &selection.storage_init {
        ui.message(&format!("Storage setup: argos {}", args.join(" ")))?;
    }
    ui.message(
        "Choose what remains in this terminal. q or cancel exits without starting services.",
    )?;
    let display = if target == "--tui" {
        prompt!("Terminal display [tui/cancel]", "tui", |v| matches!(
            v,
            "tui" | "cancel"
        ))
    } else {
        prompt!("Terminal display [tui/logs/cancel]", "logs", |v| {
            if v == "tui" && adopted && existing != Some("server") {
                return false;
            }
            matches!(v, "tui" | "logs" | "cancel")
        })
    };
    if display == "cancel" {
        return Ok(None);
    }
    if target != "--tui" {
        selection.args.extend(["--terminal".into(), display]);
    }
    Ok(Some(selection))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn choose(input: &str, existing: Option<&str>) -> Selection {
        select(&mut input.as_bytes(), &mut Vec::new(), existing)
            .unwrap()
            .unwrap()
    }
    #[test]
    fn atlas_classifier_check_leaves_services_available() {
        for needs_seed in [false, true] {
            let mut input = &b"5\n"[..];
            let mut output = Vec::new();
            let selection = configure_launcher(
                &mut Lines {
                    input: &mut input,
                    output: &mut output,
                },
                None,
                needs_seed,
            )
            .expect("wizard")
            .expect("readiness check");
            assert_eq!(selection.args, ["argos", "classifier", "status"]);
            assert!(selection.storage_init.is_none());
            let output = String::from_utf8(output).unwrap();
            for choice in [
                "1. TUI",
                "2. API Server",
                "3. Web",
                "5. Check Atlas classifier",
            ] {
                assert!(output.contains(choice), "{output}");
            }
            assert!(!output.contains("CSV path"));
        }
    }

    #[cfg(feature = "classifier")]
    #[test]
    fn legacy_training_uses_selected_csv_and_needs_no_storage_setup() {
        for existing in [None, Some("embedded")] {
            let automatic = choose("6\n\n", existing);
            assert_eq!(
                automatic.args,
                [
                    "argos",
                    "classifier",
                    "train",
                    "--activate-synthetic-baseline"
                ]
            );
            assert!(automatic.storage_init.is_none());
            let explicit = choose("6\n\"/fixture data/articles.csv\"\n", existing);
            assert_eq!(
                &explicit.args[3..],
                [
                    "--activate-synthetic-baseline",
                    "--file",
                    "/fixture data/articles.csv"
                ]
            );
            assert!(explicit.storage_init.is_none());
        }
        for input in ["6\nq\n", "6\n"] {
            assert!(select(&mut input.as_bytes(), &mut Vec::new(), None)
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn actor_csv_path_is_passed_as_a_literal_argument() {
        let mut input = &b"4\n\"/fixture data/actors.csv\"\n"[..];
        let mut output = Vec::new();
        let selected = configure_launcher(
            &mut Lines {
                input: &mut input,
                output: &mut output,
            },
            Some("embedded"),
            true,
        )
        .unwrap()
        .unwrap();
        assert!(selected.storage_init.is_none());
        assert_eq!(
            selected.args,
            [
                "argos",
                "storage",
                "seed-actors",
                "--file",
                "/fixture data/actors.csv"
            ]
        );
    }

    #[test]
    fn seed_service_is_conditional_and_dispatches_without_stack_startup() {
        let mut input = std::io::Cursor::new(b"4\n\n");
        let mut output = Vec::new();
        let selected = configure_launcher(
            &mut Lines {
                input: &mut input,
                output: &mut output,
            },
            Some("embedded"),
            true,
        )
        .expect("prompt")
        .expect("selection");
        assert_eq!(selected.args, vec!["argos", "storage", "seed-actors"]);
        assert!(selected.storage_init.is_none());
        assert!(String::from_utf8(output)
            .expect("output")
            .contains("Seed DB"));
        let mut input = std::io::Cursor::new(b"4\nq\n");
        let mut output = Vec::new();
        assert!(configure_launcher(
            &mut Lines {
                input: &mut input,
                output: &mut output
            },
            Some("embedded"),
            false,
        )
        .expect("prompt")
        .is_none());
        assert!(!String::from_utf8(output)
            .expect("output")
            .contains("Seed DB"));
    }

    #[test]
    fn tui_keeps_existing_storage_and_server_initializes_new_storage() {
        let tui = choose("1\n\n\n", Some("embedded"));
        assert_eq!(tui.args, ["argos", "stack", "up", "--tui"]);
        assert_eq!(tui.storage_init, None);
        let api = choose("2\n9090\nserver\n9010\nlogs\n", None);
        assert_eq!(
            api.args,
            [
                "argos",
                "stack",
                "up",
                "--server",
                "--api-port",
                "9090",
                "--terminal",
                "logs"
            ]
        );
        assert_eq!(
            api.storage_init.unwrap(),
            ["storage", "init", "--mode", "server", "--port", "9010"]
        );
    }
    #[test]
    fn web_display_is_the_last_question_and_adoption_uses_logs() {
        let mut output = Vec::new();
        let web = select(
            &mut &b"3\n\n\n\n\n\ntui\n"[..],
            &mut output,
            Some("embedded"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(&web.args[web.args.len() - 2..], ["--terminal", "tui"]);
        assert!(String::from_utf8(output)
            .unwrap()
            .ends_with("Terminal display [tui/logs/cancel] [logs]: "));
        let adopted = choose(
            "web\n4000\ninstalled\nconnect\nhttp://localhost:9000\ntui\nlogs\n",
            None,
        );
        assert_eq!(
            &adopted.args[adopted.args.len() - 4..],
            ["--api-url", "http://localhost:9000", "--terminal", "logs"]
        );
        assert_eq!(adopted.storage_init, None);
    }
    #[test]
    fn invalid_ports_collision_and_storage_switch_reprompt() {
        let web = choose(
            "3\n0\n4000\ndev\nstart\n4000\n65536\n9000\nserver\nkeep\nlogs\n",
            Some("embedded"),
        );
        assert_eq!(
            &web.args[web.args.len() - 4..],
            ["--api-port", "9000", "--terminal", "logs"]
        );
        assert_eq!(web.storage_init, None);
    }
    #[test]
    fn quit_eof_and_cancelling_display_launch_nothing() {
        for input in ["q\n", "", "1\nq\n", "1\nkeep\ncancel\n"] {
            assert!(
                select(&mut input.as_bytes(), &mut Vec::new(), Some("embedded"))
                    .unwrap()
                    .is_none()
            );
        }
    }
}
