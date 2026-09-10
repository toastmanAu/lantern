//! JSON-RPC 2.0 over HTTP.
//!
//! One client serves every backend kind: light clients and full nodes speak
//! the same envelope, and the shared indexer methods take the same shapes.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::BackendError;

#[derive(Debug, Serialize)]
pub(crate) struct Request<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: Value,
}

impl<'a> Request<'a> {
    pub(crate) const fn new(id: u64, method: &'a str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method,
            params,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RpcErrorBody {
    pub(crate) code: i64,
    pub(crate) message: String,
}

/// The result field stays a `Value` so a legitimate `null` (as
/// `get_indexer_tip` returns when the indexer is off) is distinguishable
/// from a missing field and can deserialize into `Option<T>`.
#[derive(Debug, Deserialize)]
pub(crate) struct Response {
    #[serde(default)]
    pub(crate) result: Value,
    #[serde(default)]
    pub(crate) error: Option<RpcErrorBody>,
}

/// A JSON-RPC client for one endpoint.
#[derive(Debug)]
pub struct RpcClient {
    http: reqwest::Client,
    url: String,
    next_id: AtomicU64,
}

impl RpcClient {
    /// Build a client. `timeout` bounds a single request, not a sync.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Transport`] if the underlying HTTP client
    /// cannot be built.
    pub fn new(url: impl Into<String>, timeout: Duration) -> Result<Self, BackendError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| BackendError::Transport(e.to_string()))?;
        Ok(Self {
            http,
            url: url.into(),
            next_id: AtomicU64::new(1),
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// Issue one call and decode its result.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Timeout`] if the request exceeds the
    /// configured timeout, [`BackendError::Transport`] for any other
    /// transport or decoding failure, and [`BackendError::Rpc`] if the node
    /// itself reports an error.
    pub async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, BackendError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = Request::new(id, method, params);
        let response = self
            .http
            .post(&self.url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    BackendError::Timeout
                } else {
                    BackendError::Transport(e.to_string())
                }
            })?;
        let body: Response = response
            .json()
            .await
            .map_err(|e| BackendError::Transport(e.to_string()))?;
        if let Some(err) = body.error {
            return Err(BackendError::Rpc {
                code: err.code,
                message: err.message,
            });
        }
        serde_json::from_value(body.result)
            .map_err(|e| BackendError::Transport(format!("could not decode {method} result: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::{Request, Response};

    #[test]
    fn a_request_carries_the_2_0_envelope() {
        let req = Request::new(7, "get_tip_header", serde_json::json!([]));
        let json = serde_json::to_value(&req).expect("serialises");
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 7);
        assert_eq!(json["method"], "get_tip_header");
        assert_eq!(json["params"], serde_json::json!([]));
    }

    #[test]
    fn a_success_response_yields_its_result() {
        let raw = serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": {"number": "0x1"}});
        let resp: Response = serde_json::from_value(raw).expect("deserialises");
        assert!(resp.error.is_none());
        assert_eq!(resp.result["number"], "0x1");
    }

    #[test]
    fn a_null_result_is_a_value_not_an_absence() {
        // `get_indexer_tip` legitimately returns null; that must survive as
        // Value::Null so `Option<Tip>` can deserialize from it.
        let raw = serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": null});
        let resp: Response = serde_json::from_value(raw).expect("deserialises");
        assert!(resp.error.is_none());
        assert!(resp.result.is_null());
        let tip: Option<crate::indexer::Tip> =
            serde_json::from_value(resp.result).expect("null deserialises to None");
        assert!(tip.is_none());
    }

    #[test]
    fn an_error_response_is_recognised() {
        let raw = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "error": {"code": -32601, "message": "Method not found"}
        });
        let resp: Response = serde_json::from_value(raw).expect("deserialises");
        let err = resp.error.expect("has an error");
        assert_eq!(err.code, -32601);
        assert_eq!(err.message, "Method not found");
    }
}
