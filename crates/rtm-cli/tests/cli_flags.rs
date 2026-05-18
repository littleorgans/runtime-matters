use std::process::Command;

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
