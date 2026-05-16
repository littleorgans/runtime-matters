use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=RTM_CLAUDE_PATH");

    if let Ok(path) = std::env::var("RTM_CLAUDE_PATH") {
        println!("cargo:rustc-env=RTM_COMPILED_CLAUDE_PATH={path}");
        return;
    }

    let Ok(output) = Command::new("which").arg("claude").output() else {
        return;
    };
    if !output.status.success() {
        return;
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !path.is_empty() {
        println!("cargo:rustc-env=RTM_COMPILED_CLAUDE_PATH={path}");
    }
}
