//! The supervised backend, driven against the stub light client.
#![cfg(feature = "testing")]

use std::time::Duration;

use lantern_chain_backend::backends::EmbeddedLight;
use lantern_chain_backend::supervisor::{RestartPolicy, SupervisorConfig};
use lantern_chain_backend::{BackendError, ChainBackend};
use lantern_sdk_schema::{BackendKind, BackendStatus, Network};

fn config(dir: &std::path::Path) -> SupervisorConfig {
    SupervisorConfig {
        binary: env!("CARGO_BIN_EXE_fake_light_client").into(),
        data_dir: dir.to_path_buf(),
        network: Network::Testnet,
        log_level: "info".to_string(),
        ready_timeout: Duration::from_secs(10),
        policy: RestartPolicy::default(),
        extra_env: Vec::new(),
    }
}

#[tokio::test]
async fn it_is_not_ready_until_started_and_reports_light_capabilities() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = EmbeddedLight::new(Network::Testnet, config(dir.path()));
    assert_eq!(backend.kind(), BackendKind::EmbeddedLight);
    assert!(backend.capabilities().needs_script_registration);
    assert_eq!(backend.status().await, BackendStatus::Stopped);

    let err = backend.tip_header().await.expect_err("not started yet");
    assert!(matches!(err, BackendError::NotReady), "{err:?}");
}

#[tokio::test]
async fn starting_brings_up_a_child_and_stopping_takes_it_down() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = EmbeddedLight::new(Network::Testnet, config(dir.path()));
    backend.start().await.expect("starts");
    // The stub answers local_node_info but not get_tip_header, so status
    // surfaces an RPC error rather than a fabricated tip — which is the
    // honest behaviour and proves the call reached the child.
    assert!(!matches!(backend.status().await, BackendStatus::Stopped));
    backend.stop().await.expect("stops");
    assert_eq!(backend.status().await, BackendStatus::Stopped);
}
