use super::*;

#[tokio::test]
async fn kill_unknown_session_returns_not_found() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let store_config = StoreConfig {
        db_path: temp.path().join("rtm.sqlite"),
    };
    let store = LifecycleStore::open(store_config.clone())
        .await
        .expect("store");
    let state = ServerState::new(
        DaemonConfig {
            socket_path: PathBuf::from("/tmp/rtm-test.sock"),
            shim_path: PathBuf::from("rtm"),
            log_root: temp.path().join("logs"),
            store: store_config,
            reconcile: reconcile::ReconcileConfig::default(),
        },
        store,
    )
    .expect("state");
    let request = KillRequest {
        session_id: Uuid::now_v7(),
        signal: RuntimeSignal::Term,
        grace_secs: 0,
    };

    let error = state.kill_runtime(request).await.expect_err("not found");
    assert!(error.to_string().contains("not found"), "{error}");
}
