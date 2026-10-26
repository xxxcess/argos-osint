use argos_osint_bin::stack::{loopback_url, StackCommand, Target};
use clap::Parser;
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: StackCommand,
}
#[test]
fn platform_flags_are_exclusive_and_required() {
    for args in [
        vec!["argos", "up"],
        vec!["argos", "up", "--web", "--server"],
        vec!["argos", "up", "--tui", "--web"],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
    let c = Cli::try_parse_from(["argos", "up", "--web", "--port", "3000"]).unwrap();
    let StackCommand::Up(up) = c.command else {
        panic!()
    };
    assert_eq!(up.validate().unwrap(), Target::Web);
}
#[test]
fn invalid_launches_fail_before_children() {
    for args in [
        vec!["argos", "up", "--server", "--port", "3000"],
        vec![
            "argos",
            "up",
            "--web",
            "--port",
            "3000",
            "--api-port",
            "3000",
        ],
        vec!["argos", "up", "--tui", "--api-port", "8080"],
        vec!["argos", "up", "--android"],
        vec!["argos", "up", "--web", "--api-url", "http://0.0.0.0:8080"],
    ] {
        let c = Cli::try_parse_from(args).unwrap();
        let StackCommand::Up(up) = c.command else {
            panic!()
        };
        assert!(up.validate().is_err());
    }
    assert!(loopback_url("http://127.0.0.1:8080").is_ok());
}

#[test]
fn welcome_setup_can_be_cancelled_without_initializing_storage() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    for input in ["q\n", "3\n4000\ninstalled\nstart\n9000\nembedded\ncancel\n"] {
        let home = tempfile::tempdir().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_argos"))
            .env("ARGOS_HOME", home.path())
            .env_remove("ARGOS_STORAGE_MODE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        assert!(output.contains("Welcome to Argos Venture!"));
        assert!(output.contains("2. API Server"));
        assert!(!home.path().join("storage/config.json").exists());
        assert!(!home.path().join("stack.json").exists());
    }
}

#[test]
fn terminal_display_and_browser_options_are_explicit() {
    use argos_osint_bin::stack::TerminalDisplay;
    let cli =
        Cli::try_parse_from(["argos", "up", "--web", "--terminal", "tui", "--no-open"]).unwrap();
    let StackCommand::Up(up) = cli.command else {
        panic!()
    };
    assert_eq!(up.terminal, TerminalDisplay::Tui);
    assert!(up.no_open);
    assert_eq!(up.validate().unwrap(), Target::Web);
    assert!(Cli::try_parse_from(["argos", "up", "--web", "--terminal", "invalid"]).is_err());
}
