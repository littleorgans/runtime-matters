use std::path::Path;

#[test]
fn readme_documents_docker_operator_contract() {
    let readme = repo_file("README.md");
    let body = std::fs::read_to_string(&readme).expect("README");

    for expected in [
        "Host execution is the default.",
        "--isolation docker",
        "Pattern A",
        "Headless Docker spawns",
        "/workspace",
        "Option A",
        "mcr.microsoft.com/devcontainers/base:ubuntu",
        "Distroless and Alpine/musl images",
        "are discouraged starters",
        "/bin/sh",
        "git",
        "Credential pass",
        "through is explicit.",
        "--session-id \"$SESSION_ID\"",
        "--image runtime-matters-claude:local",
        "RTM_DOCKER_IMAGE",
        "daemon",
        "startup environment default",
        "Capability changes are opt in.",
        "arm64 hosts",
        "non-root",
        "Docker init",
        "Manual detach and reconnect UX are out of scope.",
        "Pattern D",
        "Pattern E",
        "experimental",
        "Dockerfile Contract",
        "Runtime binary",
        "runtime executable on `PATH`",
        "exit code",
    ] {
        assert!(body.contains(expected), "README missing {expected:?}");
    }
}

#[test]
fn changelog_records_docker_boundaries() {
    let changelog = repo_file("CHANGELOG.md");
    let body = std::fs::read_to_string(&changelog).expect("CHANGELOG");

    for expected in [
        "experimental Docker isolation diagnostics",
        "Host execution remains the default",
        "Pattern D",
        "Pattern E",
        "Kubernetes",
        "SandboxClaim",
        "credential volume management",
        "privileged execution",
    ] {
        assert!(body.contains(expected), "CHANGELOG missing {expected:?}");
    }
}

#[test]
fn claude_dockerfile_conforms_to_contract() {
    let dockerfile = repo_file("examples/dockerfiles/claude.Dockerfile");
    let body = std::fs::read_to_string(&dockerfile).expect("Dockerfile");

    for expected in [
        "FROM mcr.microsoft.com/devcontainers/base:ubuntu",
        "USER ${USERNAME}",
        "WORKDIR /workspace",
        "git bash",
        "nodejs npm",
        "npm install -g @anthropic-ai/claude-code",
        "CMD [\"claude\"]",
    ] {
        assert!(body.contains(expected), "Dockerfile missing {expected:?}");
    }
    assert!(
        !body.contains("ENTRYPOINT"),
        "Dockerfile must not mask runtime command"
    );

    // Regression: devcontainers/base:ubuntu ships a `vscode` user at UID/GID
    // 1000, so the example Dockerfile must default the new user to a free id
    // or `groupadd` fails out of the box.
    assert!(
        !body.contains("ARG USER_UID=1000"),
        "Default USER_UID collides with the devcontainers base `vscode` user"
    );
    assert!(
        !body.contains("ARG USER_GID=1000"),
        "Default USER_GID collides with the devcontainers base `vscode` group"
    );
}

fn repo_file(path: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}
