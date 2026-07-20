use std::process::Command;

#[test]
fn agent_run_is_the_stable_cli_surface() {
    let output = Command::new(env!("CARGO_BIN_EXE_plasmate"))
        .args(["agent-run", "--help"])
        .output()
        .expect("run plasmate help");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: plasmate agent-run"));
    assert!(stdout.contains("--plan <PLAN>"));
    assert!(stdout.contains("--report <REPORT>"));
    assert!(stdout.contains("--confirm-step <CONFIRM_STEPS>"));
}
