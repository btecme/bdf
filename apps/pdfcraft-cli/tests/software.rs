//! The software commands and their authorisation, through the real `bdf-cli` (BUILD-018).

use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_bdf-cli");

fn run(args: &[&str]) -> Output {
    Command::new(BIN).args(args).output().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn update_status_reads_state_and_a_build_without_a_signing_key_says_so() {
    let o = run(&["update", "status"]);
    assert!(o.status.success(), "{}", err(&o));
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["current"], env!("CARGO_PKG_VERSION"));
    // This test build was made without BDF_UPDATE_PUBKEY: it cannot verify, so it offers nothing.
    assert_eq!(v["trust"], "missing");
    assert!(v["update_available"].is_null());
    assert_eq!(v["managed"], false, "a cargo build is not a managed portable package");
}

#[test]
fn a_build_without_a_key_refuses_to_check_and_changes_nothing() {
    let o = run(&["update", "check"]);
    assert!(!o.status.success());
    assert!(err(&o).contains("no release signing key"), "{}", err(&o));
}

#[test]
fn an_agent_route_needs_the_software_switch_for_operations_but_not_for_status() {
    let status = run(&["run", "update_status"]);
    assert!(status.status.success(), "{}", err(&status));
    for tool in ["update_apply", "update_rollback", "update_check", "windows_integration_register", "windows_integration_remove"] {
        let o = run(&["run", tool]);
        assert!(!o.status.success(), "{tool}");
        assert!(err(&o).contains("--allow-software"), "{tool}: {}", err(&o));
    }
    // With the switch the tool runs (and fails on its own terms: nothing to apply here), proving the gate was the only barrier.
    let allowed = run(&["run", "--allow-software", "update_apply"]);
    assert!(!allowed.status.success());
    assert!(!err(&allowed).contains("not authorised"), "{}", err(&allowed));
}

#[test]
fn the_switch_does_not_swallow_the_tool_name_and_unknown_subcommands_are_named() {
    let o = run(&["run", "--allow-software", "update_status"]);
    assert!(o.status.success(), "{}", err(&o));
    let o = run(&["update", "frobnicate"]);
    assert!(!o.status.success() && err(&o).contains("unknown subcommand"), "{}", err(&o));
    let o = run(&["windows", "frobnicate"]);
    assert!(!o.status.success() && err(&o).contains("unknown subcommand"), "{}", err(&o));
}

#[test]
fn windows_status_is_read_only() {
    let o = run(&["windows", "status"]);
    assert!(o.status.success(), "{}", err(&o));
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert!(v["summary"].is_string());
}
