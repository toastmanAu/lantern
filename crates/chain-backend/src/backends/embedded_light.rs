//! A light client whose process Lantern owns.
//!
//! Identical RPC to [`super::RemoteLight`]; the difference is lifecycle. The
//! port is not known until the child is up, so the RPC client is built at
//! start time rather than construction time.

use std::sync::Arc;

use async_trait::async_trait;
use ckb_jsonrpc_types::{HeaderView, Transaction, TransactionWithStatusResponse};
use ckb_types::H256;
use lantern_sdk_schema::{BackendCapabilities, BackendKind, BackendStatus, Network};
use tokio::sync::Mutex;

use crate::backend::ChainBackend;
use crate::backends::remote_light::{DEFAULT_TIMEOUT, RemoteLight};
use crate::cursor::{CellPage, Cursor};
use crate::error::BackendError;
use crate::light::LightRpc;
use crate::query::{CellQuery, WatchedScript};
use crate::supervisor::{Supervisor, SupervisorConfig, SupervisorHealth};

struct Inner {
    supervisor: Option<Supervisor>,
    rpc: Option<Arc<LightRpc>>,
    config: SupervisorConfig,
}

/// The default backend: a supervised `ckb-light-client`.
pub struct EmbeddedLight {
    network: Network,
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for EmbeddedLight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddedLight")
            .field("network", &self.network)
            .finish_non_exhaustive()
    }
}

impl EmbeddedLight {
    pub fn new(network: Network, config: SupervisorConfig) -> Self {
        Self {
            network,
            inner: Mutex::new(Inner {
                supervisor: None,
                rpc: None,
                config,
            }),
        }
    }

    /// Clone out the RPC handle, restarting a crashed child first.
    ///
    /// The lock is released before any network call, so queries do not queue
    /// behind one another.
    async fn rpc(&self) -> Result<Arc<LightRpc>, BackendError> {
        let mut inner = self.inner.lock().await;
        let Some(supervisor) = inner.supervisor.as_mut() else {
            return Err(BackendError::NotReady);
        };
        supervisor.ensure_running().await?;
        let port = supervisor.port();
        // A restart moves the port, so rebuild the client when it changes.
        let stale = inner
            .rpc
            .as_ref()
            .is_none_or(|rpc| !rpc.url().contains(&port.to_string()));
        if stale {
            inner.rpc = Some(Arc::new(LightRpc::new(
                format!("http://127.0.0.1:{port}/"),
                DEFAULT_TIMEOUT,
            )?));
        }
        inner.rpc.clone().ok_or(BackendError::NotReady)
    }
}

#[async_trait]
impl ChainBackend for EmbeddedLight {
    fn kind(&self) -> BackendKind {
        BackendKind::EmbeddedLight
    }

    fn network(&self) -> Network {
        self.network
    }

    fn capabilities(&self) -> BackendCapabilities {
        RemoteLight::light_capabilities()
    }

    async fn status(&self) -> BackendStatus {
        {
            let inner = self.inner.lock().await;
            match inner.supervisor.as_ref().map(Supervisor::health) {
                None | Some(SupervisorHealth::Stopped) => return BackendStatus::Stopped,
                Some(SupervisorHealth::CircuitOpen) => {
                    return BackendStatus::Error {
                        message: "light client restarted too many times".to_string(),
                    };
                }
                Some(SupervisorHealth::Restarting { attempt }) => {
                    return BackendStatus::Error {
                        message: format!("light client restarting (attempt {attempt})"),
                    };
                }
                Some(SupervisorHealth::Running { .. }) => {}
            }
        }
        match self.rpc().await {
            Ok(rpc) => RemoteLight::light_status(&rpc).await,
            Err(e) => BackendStatus::Error {
                message: e.to_string(),
            },
        }
    }

    async fn tip_header(&self) -> Result<HeaderView, BackendError> {
        self.rpc().await?.tip_header().await
    }

    async fn get_cells(
        &self,
        query: &CellQuery,
        after: Option<&Cursor>,
    ) -> Result<CellPage, BackendError> {
        self.rpc().await?.get_cells(query, after).await
    }

    async fn get_transaction(
        &self,
        hash: &H256,
    ) -> Result<Option<TransactionWithStatusResponse>, BackendError> {
        self.rpc().await?.get_transaction(hash).await
    }

    async fn send_transaction(&self, tx: &Transaction) -> Result<H256, BackendError> {
        self.rpc().await?.send_transaction(tx).await
    }

    async fn watch_scripts(&self, scripts: &[WatchedScript]) -> Result<(), BackendError> {
        self.rpc().await?.set_scripts_partial(scripts).await
    }

    async fn start(&self) -> Result<(), BackendError> {
        let mut inner = self.inner.lock().await;
        if inner.supervisor.is_some() {
            return Ok(());
        }
        let supervisor = Supervisor::start(inner.config.clone()).await?;
        let port = supervisor.port();
        inner.rpc = Some(Arc::new(LightRpc::new(
            format!("http://127.0.0.1:{port}/"),
            DEFAULT_TIMEOUT,
        )?));
        inner.supervisor = Some(supervisor);
        drop(inner);
        Ok(())
    }

    async fn stop(&self) -> Result<(), BackendError> {
        let mut inner = self.inner.lock().await;
        if let Some(mut supervisor) = inner.supervisor.take() {
            supervisor.stop().await?;
        }
        inner.rpc = None;
        drop(inner);
        Ok(())
    }
}
