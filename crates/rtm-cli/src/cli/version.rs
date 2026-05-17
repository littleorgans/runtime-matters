use anyhow::{Result, bail};

pub async fn run() -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let response = crate::shared::request(&socket_path, rtm_core::RuntimeRpc::Version).await?;
    match response {
        rtm_core::RuntimeResponse::Version { version } => {
            println!("{}", serde_json::to_string_pretty(&version)?);
            Ok(())
        }
        other => bail!("unexpected version response: {other:?}"),
    }
}
