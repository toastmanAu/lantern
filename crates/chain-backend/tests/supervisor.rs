//! Supervisor lifecycle, driven against a stub binary rather than the real
//! light client so the suite stays hermetic and fast.
#![cfg(feature = "testing")]

use std::time::Duration;

use lantern_chain_backend::supervisor::{Supervisor, SupervisorConfig, SupervisorHealth};
use lantern_sdk_schema::Network;

fn config(dir: &std::path::Path) -> SupervisorConfig {
    SupervisorConfig {
        binary: env!("CARGO_BIN_EXE_fake_light_client").into(),
        data_dir: dir.to_path_buf(),
        network: Network::Testnet,
        log_level: "info".to_string(),
        ready_timeout: Duration::from_secs(10),
        extra_env: Vec::new(),
    }
}

#[tokio::test]
async fn starts_becomes_ready_and_reports_its_port() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut sup = Supervisor::start(config(dir.path())).await.expect("starts");
    assert!(sup.port() > 0, "a port was allocated");
    assert!(matches!(sup.health(), SupervisorHealth::Running { .. }));

    // The port it reports is the port it is actually serving on.
    let client = lantern_chain_backend::RpcClient::new(
        format!("http://127.0.0.1:{}/", sup.port()),
        Duration::from_secs(5),
    )
    .expect("client");
    let info: serde_json::Value = client
        .call("local_node_info", serde_json::json!([]))
        .await
        .expect("answers");
    assert_eq!(info["version"], "fake");

    sup.stop().await.expect("stops");
    assert!(matches!(sup.health(), SupervisorHealth::Stopped));
}

#[tokio::test]
async fn a_process_that_never_binds_times_out_rather_than_hanging() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut cfg = config(dir.path());
    cfg.ready_timeout = Duration::from_millis(600);
    // Steer the child, not the test process: `extra_env` is passed to the
    // spawned child via `Command::envs`, so no global env mutation is
    // needed here (and none is visible to other tests running in parallel).
    cfg.extra_env = vec![("FAKE_LC_NEVER_READY".to_string(), "1".to_string())];
    let result = Supervisor::start(cfg).await;
    assert!(
        matches!(result, Err(lantern_chain_backend::BackendError::Timeout)),
        "expected a timeout"
    );
}

#[tokio::test]
async fn a_missing_binary_is_a_spawn_error_not_a_panic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut cfg = config(dir.path());
    cfg.binary = dir.path().join("definitely-not-here");
    let err = Supervisor::start(cfg).await.expect_err("must fail");
    assert!(
        matches!(err, lantern_chain_backend::BackendError::Spawn(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn two_supervisors_get_different_ports() {
    let a_dir = tempfile::tempdir().expect("tempdir");
    let b_dir = tempfile::tempdir().expect("tempdir");
    let mut a = Supervisor::start(config(a_dir.path()))
        .await
        .expect("a starts");
    let mut b = Supervisor::start(config(b_dir.path()))
        .await
        .expect("b starts");
    assert_ne!(a.port(), b.port(), "profiles must run side by side");
    a.stop().await.expect("a stops");
    b.stop().await.expect("b stops");
}

#[tokio::test]
async fn stopping_reaps_the_child_and_frees_the_port() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut sup = Supervisor::start(config(dir.path())).await.expect("starts");
    let port = sup.port();
    sup.stop().await.expect("stops");
    assert!(matches!(sup.health(), SupervisorHealth::Stopped));

    // The child is gone, so the port binds again.
    let rebind = std::net::TcpListener::bind(("127.0.0.1", port));
    assert!(rebind.is_ok(), "port {port} was not released");
}

#[tokio::test]
async fn stopping_twice_is_harmless() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut sup = Supervisor::start(config(dir.path())).await.expect("starts");
    sup.stop().await.expect("first stop");
    sup.stop().await.expect("second stop is a no-op");
    assert!(matches!(sup.health(), SupervisorHealth::Stopped));
}
