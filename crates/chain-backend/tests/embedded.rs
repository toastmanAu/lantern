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

/// The RPC `listen_address` in the generated config, which
/// `Supervisor::start` rewrites with a fresh port on every spawn. Reading it
/// is how a test tells "a new child was spawned" from "nothing happened".
fn generated_listen_address(dir: &std::path::Path) -> String {
    let path = dir.join("testnet").join("light-client.toml");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        .lines()
        .find_map(|l| l.trim().strip_prefix("listen_address = "))
        .map(|v| v.trim().trim_matches('"').to_string())
        .expect("the generated config carries an rpc listen_address")
}

#[tokio::test]
async fn start_on_an_open_breaker_respawns_instead_of_reporting_success() {
    // The supervisor is still `Some` once the breaker trips — child gone,
    // `ensure_running` refusing to retry — so an unconditional `Ok(())` made
    // a "Reconnect" button report success on every press over a dead backend.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut cfg = config(dir.path());
    cfg.policy = RestartPolicy {
        backoff_base: Duration::from_millis(10),
        backoff_cap: Duration::from_millis(40),
        breaker_threshold: 3,
        breaker_window: Duration::from_secs(60),
    };
    // Long enough to clear the readiness poll, short enough that three exits
    // land well inside the 60s breaker window.
    cfg.extra_env
        .push(("FAKE_LC_EXIT_AFTER_MS".into(), "60".into()));

    let backend = EmbeddedLight::new(Network::Testnet, cfg);
    backend.start().await.expect("starts");

    let mut opened = false;
    for _ in 0..80 {
        let _ = backend.tip_header().await;
        if matches!(
            backend.status().await,
            BackendStatus::Error { ref message } if message.contains("too many times")
        ) {
            opened = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        opened,
        "the breaker must open, or the assertion below proves nothing"
    );

    let before = generated_listen_address(dir.path());
    backend
        .start()
        .await
        .expect("start must reset the breaker rather than fail");
    let after = generated_listen_address(dir.path());
    assert_ne!(
        before, after,
        "start() over an open breaker must spawn a fresh child (new port, \
         regenerated config), not return Ok having done nothing"
    );

    backend.stop().await.expect("stops");
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
