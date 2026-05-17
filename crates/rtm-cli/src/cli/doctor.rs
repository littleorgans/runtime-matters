use anyhow::{Result, bail};

pub async fn run() -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let response = crate::shared::request(&socket_path, rtm_core::RuntimeRpc::Watchers).await?;
    match response {
        rtm_core::RuntimeResponse::Watchers { watchers } => {
            println!(
                "kqueue_watchers={} shim_sockets={}",
                watchers.kqueue_watchers, watchers.shim_sockets
            );
            Ok(())
        }
        other => bail!("unexpected watchers response: {other:?}"),
    }
}
