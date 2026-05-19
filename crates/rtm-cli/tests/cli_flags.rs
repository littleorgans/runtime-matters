use std::process::Command;

use uuid::Uuid;

#[test]
fn root_version_flag_prints_runtime_matters_package_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_rtm"))
        .arg("--version")
        .output()
        .expect("rtm --version");

    assert!(output.status.success(), "rtm --version failed: {output:?}");
    assert!(output.stderr.is_empty(), "stderr was not empty: {output:?}");

    let stdout = String::from_utf8(output.stdout).expect("version output utf8");
    let expected = format!("runtime-matters {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(stdout, expected);
}

#[test]
fn spawn_help_documents_cwd_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_rtm"))
        .args(["spawn", "--help"])
        .output()
        .expect("rtm spawn --help");

    assert!(
        output.status.success(),
        "rtm spawn --help failed: {output:?}"
    );
    assert!(output.stderr.is_empty(), "stderr was not empty: {output:?}");

    let stdout = String::from_utf8(output.stdout).expect("help output utf8");
    assert!(stdout.contains("--cwd <PATH>"), "{stdout}");
}

#[test]
fn spawn_cwd_flag_rejects_missing_path_before_daemon_request() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let missing = temp.path().join("missing");
    let output = spawn_with_cwd(&missing);

    assert!(!output.status.success(), "spawn unexpectedly succeeded");
    let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
    assert!(
        stderr.contains(&format!("spawn cwd does not exist: {}", missing.display())),
        "{stderr}"
    );
}

#[test]
fn spawn_cwd_flag_rejects_file_path_before_daemon_request() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let file = temp.path().join("file");
    std::fs::write(&file, "").expect("file");
    let file = std::fs::canonicalize(file).expect("canonical file");
    let output = spawn_with_cwd(&file);

    assert!(!output.status.success(), "spawn unexpectedly succeeded");
    let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
    assert!(
        stderr.contains(&format!("spawn cwd is not a directory: {}", file.display())),
        "{stderr}"
    );
}

fn spawn_with_cwd(path: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rtm"))
        .args([
            "spawn",
            "--runtime",
            "claude",
            "--session-id",
            &Uuid::now_v7().to_string(),
            "--target",
            "headless",
            "--cwd",
        ])
        .arg(path)
        .output()
        .expect("rtm spawn")
}
