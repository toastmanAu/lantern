//! Lifecycle of the bundled light-client process.
//!
//! Lantern owns this process: it generates the config, allocates the ports,
//! waits for readiness, forwards the logs, and reaps it on exit.

use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use lantern_sdk_schema::Network;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

use crate::config::LightClientConfig;
use crate::error::BackendError;
use crate::rpc::RpcClient;

/// What the supervisor knows about its child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorHealth {
    Running {
        restarts: u32,
    },
    Restarting {
        attempt: u32,
    },
    /// Too many crashes too quickly; auto-restart has given up.
    CircuitOpen,
    Stopped,
}

/// Everything needed to spawn the light client.
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub binary: PathBuf,
    pub data_dir: PathBuf,
    pub network: Network,
    pub log_level: String,
    pub ready_timeout: Duration,
    /// Extra environment variables for the child process.
    ///
    /// Tests use this to steer the stub binary's failure modes (see
    /// `fake_light_client`) instead of mutating the parent process's
    /// environment with `std::env::set_var`, which is unsafe under this
    /// crate's edition and racy across tests running in parallel regardless.
    pub extra_env: Vec<(String, String)>,
}

/// A running light-client process.
pub struct Supervisor {
    child: Option<Child>,
    rpc_port: u16,
    health: SupervisorHealth,
    log_tasks: Vec<JoinHandle<()>>,
    config: SupervisorConfig,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor")
            .field("rpc_port", &self.rpc_port)
            .field("health", &self.health)
            .finish_non_exhaustive()
    }
}

/// Ask the OS for a free port, then release it.
///
/// Inherently a race: something else could take the port before the child
/// binds it. The readiness poll is what catches that, and the caller retries
/// with a fresh port.
fn free_port() -> Result<u16, BackendError> {
    let listener = StdTcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

impl Supervisor {
    /// Generate a config, spawn the child, and wait until it answers.
    pub async fn start(config: SupervisorConfig) -> Result<Self, BackendError> {
        let rpc_port = free_port()?;
        let p2p_port = free_port()?;

        let lc_config = LightClientConfig {
            data_dir: config.data_dir.clone(),
            network: config.network,
            rpc_port,
            p2p_port,
        };
        let config_path = config.data_dir.join("light-client.toml");
        lc_config.write_to(&config_path)?;

        let mut child = Command::new(&config.binary)
            .arg("run")
            .arg("--config-file")
            .arg(&config_path)
            .env("RUST_LOG", &config.log_level)
            .envs(config.extra_env.iter().cloned())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| BackendError::Spawn(format!("{}: {e}", config.binary.display())))?;

        let pid = child.id().unwrap_or_default();
        let mut log_tasks = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            log_tasks.push(forward(stdout, pid, "stdout"));
        }
        if let Some(stderr) = child.stderr.take() {
            log_tasks.push(forward(stderr, pid, "stderr"));
        }

        let mut supervisor = Self {
            child: Some(child),
            rpc_port,
            health: SupervisorHealth::Running { restarts: 0 },
            log_tasks,
            config,
        };

        if let Err(e) = supervisor.await_ready().await {
            // Never leave an unreachable child behind.
            let _ = supervisor.stop().await;
            return Err(e);
        }
        Ok(supervisor)
    }

    pub const fn port(&self) -> u16 {
        self.rpc_port
    }

    pub fn health(&self) -> SupervisorHealth {
        self.health.clone()
    }

    /// Kill the child, stop forwarding its logs, and mark this supervisor
    /// stopped.
    ///
    /// This is a temporary placeholder for Task 13: it does not attempt a
    /// graceful shutdown (SIGTERM before SIGKILL) and knows nothing about
    /// restart or circuit-breaker policy. Task 14 replaces it with the real
    /// implementation; it always returns `Ok(())` because there is currently
    /// nothing here that can meaningfully fail from the caller's point of
    /// view.
    pub async fn stop(&mut self) -> Result<(), BackendError> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        for task in self.log_tasks.drain(..) {
            task.abort();
        }
        self.health = SupervisorHealth::Stopped;
        Ok(())
    }

    /// Poll `local_node_info` until it answers or the deadline passes.
    async fn await_ready(&mut self) -> Result<(), BackendError> {
        let url = format!("http://127.0.0.1:{}/", self.rpc_port);
        let client = RpcClient::new(url, Duration::from_secs(2))?;
        let deadline = Instant::now() + self.config.ready_timeout;
        let mut backoff = Duration::from_millis(100);
        loop {
            if let Some(child) = self.child.as_mut()
                && let Ok(Some(status)) = child.try_wait()
            {
                return Err(BackendError::Spawn(format!(
                    "light client exited during startup with {status}"
                )));
            }
            if client
                .call::<serde_json::Value>("local_node_info", serde_json::json!([]))
                .await
                .is_ok()
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(BackendError::Timeout);
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(2));
        }
    }
}

/// Re-emit a child stream through `tracing`, tagged with its PID.
fn forward<R>(stream: R, pid: u32, name: &'static str) -> JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "lantern::light_client", pid, stream = name, "{line}");
        }
    })
}
