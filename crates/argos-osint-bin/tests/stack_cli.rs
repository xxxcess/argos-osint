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

#[test]
fn abandoned_supervisor_state_is_reported_and_recovered() {
    use std::process::Command;
    let home = tempfile::tempdir().unwrap();
    let stack_json = home.path().join("stack.json");
    let state = serde_json::json!({
        "target": "Web",
        "profile": "Dev",
        "terminal": "Logs",
        "web_port": 3000,
        "instance": "test_instance",
        "token": "token123",
        "control_port": 61234,
        "api_url": "http://127.0.0.1:8080",
        "api_instance": "api_inst",
        "adopted": false,
        "supervisor_pid": 999999,
        "api_pid": null,
        "frontend_pid": null
    });
    std::fs::write(&stack_json, serde_json::to_vec(&state).unwrap()).unwrap();

    // 1. stack status detects abandoned state without crashing
    let status_out = Command::new(env!("CARGO_BIN_EXE_argos"))
        .env("ARGOS_HOME", home.path())
        .args(["stack", "status"])
        .output()
        .unwrap();
    assert!(status_out.status.success());
    let stdout = String::from_utf8(status_out.stdout).unwrap();
    assert!(stdout.contains("Abandoned supervisor state"));
    assert!(stdout.contains("Run 'argos stack down' to clean up"));
    assert!(stack_json.exists());

    // 2. stack down recovers abandoned state and removes stack.json
    let down_out = Command::new(env!("CARGO_BIN_EXE_argos"))
        .env("ARGOS_HOME", home.path())
        .args(["stack", "down", "--web"])
        .output()
        .unwrap();
    assert!(down_out.status.success());
    let down_stdout = String::from_utf8(down_out.stdout).unwrap();
    assert!(down_stdout.contains("Cleaned up abandoned supervisor state"));
    assert!(!stack_json.exists());
}

#[test]
fn corrupted_supervisor_state_is_cleaned_up() {
    use std::process::Command;
    let home = tempfile::tempdir().unwrap();
    let stack_json = home.path().join("stack.json");
    std::fs::write(&stack_json, b"corrupted-non-json").unwrap();

    // 1. stack status reports corrupted state
    let status_out = Command::new(env!("CARGO_BIN_EXE_argos"))
        .env("ARGOS_HOME", home.path())
        .args(["stack", "status"])
        .output()
        .unwrap();
    assert!(status_out.status.success());
    let stdout = String::from_utf8(status_out.stdout).unwrap();
    assert!(stdout.contains("Corrupted supervisor state"));

    // 2. stack down removes corrupted file
    let down_out = Command::new(env!("CARGO_BIN_EXE_argos"))
        .env("ARGOS_HOME", home.path())
        .args(["stack", "down", "--web"])
        .output()
        .unwrap();
    assert!(down_out.status.success());
    assert!(!stack_json.exists());
}
