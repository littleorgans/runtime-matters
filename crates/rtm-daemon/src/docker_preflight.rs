use anyhow::Result;
use serde::Deserialize;
use tokio::process::Command;

use crate::error::RuntimeFailure;

const DEFAULT_DOCKER_IMAGE: &str = "runtime-matters-agent:latest";
const RTM_DOCKER_IMAGE: &str = "RTM_DOCKER_IMAGE";
const RTM_DOCKER_ALLOW_ROOT_IMAGE_USER: &str = "RTM_DOCKER_ALLOW_ROOT_IMAGE_USER";
const RTM_DOCKER_ALLOW_ARM64_MANIFEST_ESCAPE: &str = "RTM_DOCKER_ALLOW_ARM64_MANIFEST_ESCAPE";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockerPreflightConfig {
    image: String,
    allow_root_image_user: bool,
    allow_arm64_manifest_escape: bool,
}

impl DockerPreflightConfig {
    pub fn from_env() -> Self {
        Self {
            image: std::env::var(RTM_DOCKER_IMAGE)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_DOCKER_IMAGE.to_owned()),
            allow_root_image_user: env_flag(RTM_DOCKER_ALLOW_ROOT_IMAGE_USER),
            allow_arm64_manifest_escape: env_flag(RTM_DOCKER_ALLOW_ARM64_MANIFEST_ESCAPE),
        }
    }

    #[cfg(test)]
    pub(crate) fn new(
        image: impl Into<String>,
        allow_root_image_user: bool,
        allow_arm64_manifest_escape: bool,
    ) -> Self {
        Self {
            image: image.into(),
            allow_root_image_user,
            allow_arm64_manifest_escape,
        }
    }

    pub(crate) fn image(&self) -> &str {
        &self.image
    }

    pub(crate) fn allows_root_image_user(&self) -> bool {
        self.allow_root_image_user
    }

    pub(crate) fn allows_arm64_manifest_escape(&self) -> bool {
        self.allow_arm64_manifest_escape
    }
}

impl Default for DockerPreflightConfig {
    fn default() -> Self {
        Self {
            image: DEFAULT_DOCKER_IMAGE.to_owned(),
            allow_root_image_user: false,
            allow_arm64_manifest_escape: false,
        }
    }
}

pub(crate) trait DockerImageInspector {
    async fn ensure_available(&self) -> Result<()>;
    async fn image_user(&self, image: &str) -> Result<Option<String>>;
    async fn arm64_manifest_available(&self, image: &str) -> Result<bool>;
}

pub(crate) struct DockerCliInspector;

impl DockerImageInspector for DockerCliInspector {
    async fn ensure_available(&self) -> Result<()> {
        let output = Command::new("docker")
            .arg("version")
            .arg("--format")
            .arg("{{.Server.Version}}")
            .output()
            .await
            .map_err(|error| RuntimeFailure::docker_unavailable(error.to_string()))?;

        if output.status.success() {
            return Ok(());
        }

        Err(RuntimeFailure::docker_unavailable(command_error_message(
            &output.stderr,
            "docker version failed without stderr",
        )))
    }

    async fn image_user(&self, image: &str) -> Result<Option<String>> {
        let output = Command::new("docker")
            .arg("image")
            .arg("inspect")
            .arg(image)
            .arg("--format")
            .arg("{{json .Config.User}}")
            .output()
            .await
            .map_err(|error| RuntimeFailure::docker_image_unavailable(error.to_string()))?;

        if !output.status.success() {
            return Err(RuntimeFailure::docker_image_unavailable(
                command_error_message(&output.stderr, "docker image inspect failed without stderr"),
            ));
        }

        let raw = String::from_utf8_lossy(&output.stdout);
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == "null" {
            return Ok(None);
        }
        let user = serde_json::from_str::<String>(trimmed).map_err(|error| {
            RuntimeFailure::docker_image_metadata_unavailable(format!(
                "docker image inspect returned invalid user metadata: {error}"
            ))
        })?;
        Ok(non_empty(user))
    }

    async fn arm64_manifest_available(&self, image: &str) -> Result<bool> {
        let output = Command::new("docker")
            .arg("manifest")
            .arg("inspect")
            .arg(image)
            .output()
            .await
            .map_err(|error| {
                RuntimeFailure::docker_image_metadata_unavailable(error.to_string())
            })?;

        if !output.status.success() {
            return Err(RuntimeFailure::docker_image_metadata_unavailable(
                command_error_message(
                    &output.stderr,
                    "docker manifest inspect failed without stderr",
                ),
            ));
        }

        manifest_has_arm64(&output.stdout)
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn command_error_message(stderr: &[u8], fallback: &str) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    if message.is_empty() {
        fallback.to_owned()
    } else {
        message
    }
}

fn manifest_has_arm64(stdout: &[u8]) -> Result<bool> {
    let manifest = serde_json::from_slice::<DockerManifest>(stdout).map_err(|error| {
        RuntimeFailure::docker_image_metadata_unavailable(format!(
            "docker manifest inspect returned invalid metadata: {error}"
        ))
    })?;
    Ok(match manifest.manifests {
        Some(manifests) => manifests
            .iter()
            .any(|manifest| manifest.platform.architecture == "arm64"),
        None => true,
    })
}

#[derive(Deserialize)]
struct DockerManifest {
    manifests: Option<Vec<DockerManifestDescriptor>>,
}

#[derive(Deserialize)]
struct DockerManifestDescriptor {
    platform: DockerPlatform,
}

#[derive(Deserialize)]
struct DockerPlatform {
    architecture: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_list_reports_arm64_availability() {
        let manifest = br#"{
            "manifests": [
                { "platform": { "architecture": "amd64" } },
                { "platform": { "architecture": "arm64" } }
            ]
        }"#;

        assert!(manifest_has_arm64(manifest).expect("manifest"));
    }

    #[test]
    fn manifest_list_reports_known_arm64_absence() {
        let manifest = br#"{
            "manifests": [
                { "platform": { "architecture": "amd64" } }
            ]
        }"#;

        assert!(!manifest_has_arm64(manifest).expect("manifest"));
    }

    #[test]
    fn single_platform_manifest_is_not_a_known_arm64_absence() {
        let manifest = br#"{ "schemaVersion": 2 }"#;

        assert!(manifest_has_arm64(manifest).expect("manifest"));
    }
}
