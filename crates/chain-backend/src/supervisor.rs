//! Lifecycle of the bundled light-client process.
//!
//! Lantern owns this process: it generates the config, allocates the ports,
//! waits for readiness, forwards the logs, and reaps it on exit.

use std::collections::VecDeque;
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
    /// How aggressively a crashed child is brought back.
    pub policy: RestartPolicy,
    /// Extra environment variables for the child process.
    ///
    /// Tests use this to steer the stub binary's failure modes (see
    /// `fake_light_client`) instead of mutating the parent process's
    /// environment with `std::env::set_var`, which is unsafe under this
    /// crate's edition and racy across tests running in parallel regardless.
    pub extra_env: Vec<(String, String)>,
}

/// How aggressively a crashed child is brought back.
#[derive(Debug, Clone)]
pub struct RestartPolicy {
    pub backoff_base: Duration,
    pub backoff_cap: Duration,
    /// Exits within `breaker_window` that trip the breaker.
    pub breaker_threshold: u32,
    pub breaker_window: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        // Spec §5: 1s, 2s, 4s, 8s capped at 30s; five crashes in sixty
        // seconds stops auto-restart and surfaces the failure.
        Self {
            backoff_base: Duration::from_secs(1),
            backoff_cap: Duration::from_secs(30),
            breaker_threshold: 5,
            breaker_window: Duration::from_secs(60),
        }
    }
}

/// How the last shutdown actually went.
///
/// This is what makes the graceful path falsifiable: a child that receives
/// `SIGTERM` exits inside the grace window, while one that never receives it
/// keeps serving until the window elapses and gets killed. Without this
/// distinction, removing the signal would leave every test still green.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownKind {
    /// Exited on its own after `SIGTERM`, within the grace period.
    Graceful,
    /// Ignored or never received the signal and had to be killed.
    Forced,
}

/// A running light-client process.
pub struct Supervisor {
    child: Option<Child>,
    rpc_port: u16,
    health: SupervisorHealth,
    log_tasks: Vec<JoinHandle<()>>,
    config: SupervisorConfig,
    last_shutdown: Option<ShutdownKind>,
    /// Timestamps of recent exits, used to detect a fast-crash loop.
    exits: VecDeque<Instant>,
    /// When the next restart attempt is allowed, set by backoff.
    next_attempt_at: Option<Instant>,
    /// How many times this supervisor has restarted its child.
    restarts: u32,
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

/// How long a child gets to exit after `SIGTERM` before it is killed.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

#[cfg(unix)]
fn request_termination(pid: u32) -> Result<(), BackendError> {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    // `kill` is a safe wrapper, so the workspace's unsafe ban is untouched.
    let raw = i32::try_from(pid)
        .map_err(|_| BackendError::Spawn(format!("pid {pid} is out of range")))?;
    kill(Pid::from_raw(raw), Signal::SIGTERM)
        .map_err(|e| BackendError::Spawn(format!("could not signal {pid}: {e}")))
}

#[cfg(not(unix))]
const fn request_termination(_pid: u32) -> Result<(), BackendError> {
    // Windows has no SIGTERM; the caller falls through to a hard kill.
    Ok(())
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
            last_shutdown: None,
            exits: VecDeque::new(),
            next_attempt_at: None,
            restarts: 0,
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

    /// How the most recent call to `stop` actually shut the child down.
    ///
    /// `None` until `stop` has been called at least once.
    pub const fn last_shutdown(&self) -> Option<ShutdownKind> {
        self.last_shutdown
    }

    /// Stop the child: `SIGTERM`, a grace period, then `SIGKILL`.
    ///
    /// On Unix the child is asked to exit cleanly first, so its store isn't
    /// left mid-write, and only then does it get a grace window to act on
    /// the signal. Windows has no `SIGTERM` — and any Unix child that could
    /// not be signalled either — so those go straight to a hard kill instead
    /// of waiting out a grace period for a signal that was never sent.
    /// Either way the child is always reaped, log forwarding is always
    /// aborted, and health always ends at `Stopped`.
    ///
    /// Idempotent — calling it on an already-stopped supervisor succeeds and
    /// leaves `last_shutdown` at whatever the first call recorded.
    pub async fn stop(&mut self) -> Result<(), BackendError> {
        for task in self.log_tasks.drain(..) {
            task.abort();
        }
        let Some(mut child) = self.child.take() else {
            self.health = SupervisorHealth::Stopped;
            return Ok(());
        };

        let mut signalled = false;
        if cfg!(unix)
            && let Some(pid) = child.id()
        {
            match request_termination(pid) {
                Ok(()) => signalled = true,
                Err(e) => tracing::debug!("SIGTERM failed, falling through to kill: {e}"),
            }
        }

        let shutdown_kind = if signalled {
            if let Ok(Ok(status)) = tokio::time::timeout(SHUTDOWN_GRACE, child.wait()).await {
                tracing::debug!("light client exited with {status}");
                ShutdownKind::Graceful
            } else {
                tracing::warn!("light client did not exit in time; killing");
                let _ = child.kill().await;
                let _ = child.wait().await;
                ShutdownKind::Forced
            }
        } else {
            let _ = child.kill().await;
            let _ = child.wait().await;
            ShutdownKind::Forced
        };

        self.last_shutdown = Some(shutdown_kind);
        self.health = SupervisorHealth::Stopped;
        Ok(())
    }

    /// Make sure a child is running, restarting a crashed one within policy.
    ///
    /// Called before serving a query and when reporting status, rather than
    /// from a background watchdog: no shared mutable state, and every path is
    /// deterministic to test.
    pub async fn ensure_running(&mut self) -> Result<(), BackendError> {
        if matches!(self.health, SupervisorHealth::CircuitOpen) {
            return Err(BackendError::Spawn(
                "light client restarted too many times; not retrying".into(),
            ));
        }

        // Still alive? Nothing to do.
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(None) => return Ok(()),
                Ok(Some(status)) => {
                    tracing::warn!("light client exited with {status}");
                    self.record_exit();
                }
                Err(e) => return Err(BackendError::Spawn(e.to_string())),
            }
        }
        self.child = None;

        if matches!(self.health, SupervisorHealth::CircuitOpen) {
            return Err(BackendError::Spawn(
                "light client restarted too many times; not retrying".into(),
            ));
        }

        if let Some(at) = self.next_attempt_at
            && Instant::now() < at
        {
            self.health = SupervisorHealth::Restarting {
                attempt: self.restarts,
            };
            return Err(BackendError::NotReady);
        }

        self.restarts = self.restarts.saturating_add(1);
        match Self::start(self.config.clone()).await {
            Ok(replacement) => {
                self.adopt(replacement);
                Ok(())
            }
            Err(e) => {
                // A restart that fails to come up is just as much a crash as
                // one that comes up and dies: it must count toward the
                // breaker and arm the backoff, or a permanently broken
                // binary is retried on every single call with no throttle.
                self.record_exit();
                Err(e)
            }
        }
    }

    /// Note a crash and open the breaker if they are coming too fast.
    fn record_exit(&mut self) {
        let now = Instant::now();
        self.exits.push_back(now);
        while let Some(front) = self.exits.front() {
            if now.duration_since(*front) > self.config.policy.breaker_window {
                self.exits.pop_front();
            } else {
                break;
            }
        }
        if u32::try_from(self.exits.len()).unwrap_or(u32::MAX)
            >= self.config.policy.breaker_threshold
        {
            tracing::error!(
                "light client crashed {} times within {:?}; giving up",
                self.exits.len(),
                self.config.policy.breaker_window
            );
            self.health = SupervisorHealth::CircuitOpen;
            return;
        }
        let shift = self.restarts.min(16);
        let backoff = self
            .config
            .policy
            .backoff_base
            .saturating_mul(1_u32 << shift)
            .min(self.config.policy.backoff_cap);
        self.next_attempt_at = Some(now + backoff);
    }

    /// Take over a freshly started supervisor's child, keeping crash history.
    ///
    /// `other` is a `Supervisor` returned by `start` inside `ensure_running`;
    /// it exists only to carry the new child out of that call. Only the
    /// fields describing the *live process* move across — `child`, the
    /// port it bound, and the log-forwarding tasks reading its stdout and
    /// stderr. `exits`, `restarts` and `last_shutdown` stay on `self`
    /// because they are this supervisor's own history, not the donor's:
    /// `other` was never restarted and never stopped, so its versions of
    /// those fields are just their initial values and would erase what
    /// `self` has recorded. Taking `child` (via `Option::take`) and
    /// `log_tasks` (via `mem::take`) leaves `other` inert, so when it is
    /// dropped at the end of `ensure_running` its `Child` field is `None`
    /// and `kill_on_drop` has nothing left to kill.
    fn adopt(&mut self, mut other: Self) {
        self.child = other.child.take();
        self.rpc_port = other.rpc_port;
        self.log_tasks = std::mem::take(&mut other.log_tasks);
        self.next_attempt_at = None;
        self.health = SupervisorHealth::Running {
            restarts: self.restarts,
        };
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
