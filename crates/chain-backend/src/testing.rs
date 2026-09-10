//! An in-process fake CKB node for tests.
//!
//! Hand-rolled HTTP/1.1 rather than a web framework: the crate needs no HTTP
//! server in production, and the point of this type is total control over the
//! bytes a test sees, including the terminal `"0x"` cursor a real node emits.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

type Routes = Arc<Mutex<HashMap<String, VecDeque<Reply>>>>;
type Calls = Arc<Mutex<Vec<(String, Value)>>>;

#[derive(Debug, Clone)]
enum Reply {
    Ok(Value),
    Err { code: i64, message: String },
}

/// Builds a [`FakeNode`].
#[derive(Default)]
pub struct FakeNodeBuilder {
    routes: HashMap<String, VecDeque<Reply>>,
}

impl FakeNodeBuilder {
    /// Always answer `method` with `result`.
    #[must_use]
    pub fn respond(mut self, method: &str, result: Value) -> Self {
        self.routes
            .entry(method.to_string())
            .or_default()
            .push_back(Reply::Ok(result));
        self
    }

    /// Answer successive calls to `method` with successive values. The last
    /// one repeats once the queue is down to it, so a scan can be driven to
    /// exhaustion and then poked again.
    #[must_use]
    pub fn respond_sequence(mut self, method: &str, results: Vec<Value>) -> Self {
        let queue = self.routes.entry(method.to_string()).or_default();
        for r in results {
            queue.push_back(Reply::Ok(r));
        }
        self
    }

    /// Answer `method` with a JSON-RPC error.
    #[must_use]
    pub fn fail(mut self, method: &str, code: i64, message: &str) -> Self {
        self.routes
            .entry(method.to_string())
            .or_default()
            .push_back(Reply::Err {
                code,
                message: message.to_string(),
            });
        self
    }

    /// Bind an ephemeral port and start serving.
    pub async fn start(self) -> FakeNode {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let addr = listener.local_addr().expect("local addr");
        let routes: Routes = Arc::new(Mutex::new(self.routes));
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let (stop_tx, stop_rx) = oneshot::channel();
        let served_routes = Arc::clone(&routes);
        let served_calls = Arc::clone(&calls);
        tokio::spawn(async move { serve(listener, served_routes, served_calls, stop_rx).await });
        FakeNode {
            addr,
            calls,
            stop: Some(stop_tx),
        }
    }
}

/// A fake node listening on localhost for the life of the value.
pub struct FakeNode {
    addr: SocketAddr,
    calls: Calls,
    stop: Option<oneshot::Sender<()>>,
}

impl FakeNode {
    #[must_use]
    pub fn builder() -> FakeNodeBuilder {
        FakeNodeBuilder::default()
    }

    #[must_use]
    pub fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    /// Every `(method, params)` received, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().expect("calls mutex").clone()
    }

    /// How many times `method` was called.
    #[must_use]
    pub fn call_count(&self, method: &str) -> usize {
        self.calls().iter().filter(|(m, _)| m == method).count()
    }
}

impl Drop for FakeNode {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

async fn serve(listener: TcpListener, routes: Routes, calls: Calls, stop: oneshot::Receiver<()>) {
    tokio::pin!(stop);
    loop {
        tokio::select! {
            _ = &mut stop => break,
            accepted = listener.accept() => {
                let Ok((mut socket, _)) = accepted else { continue };
                let routes = Arc::clone(&routes);
                let calls = Arc::clone(&calls);
                tokio::spawn(async move {
                    let _ = handle(&mut socket, &routes, &calls).await;
                });
            }
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Reads the request head and returns `(buffered bytes, header-end offset,
/// declared content length)`. Returns `None` if the peer closed before a full
/// head arrived.
async fn read_head(socket: &mut TcpStream) -> std::io::Result<Option<(Vec<u8>, usize, usize)>> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0_u8; 2048];
    let head_end = loop {
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_ascii_lowercase();
    let content_length = head
        .split("content-length:")
        .nth(1)
        .and_then(|rest| rest.split("\r\n").next())
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    Ok(Some((buf, head_end, content_length)))
}

async fn handle(socket: &mut TcpStream, routes: &Routes, calls: &Calls) -> std::io::Result<()> {
    let Some((mut buf, head_end, content_length)) = read_head(socket).await? else {
        return Ok(());
    };
    let mut chunk = [0_u8; 2048];
    while buf.len() < head_end + content_length {
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }

    let request: Value = serde_json::from_slice(&buf[head_end..]).unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let id = request.get("id").cloned().unwrap_or_else(|| json!(1));
    calls
        .lock()
        .expect("calls mutex")
        .push((method.clone(), params));

    let reply = {
        let mut guard = routes.lock().expect("routes mutex");
        match guard.get_mut(&method) {
            Some(queue) if queue.len() > 1 => queue.pop_front(),
            Some(queue) => queue.front().cloned(),
            None => None,
        }
    };
    let body = match reply {
        Some(Reply::Ok(result)) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Some(Reply::Err { code, message }) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
        None => json!({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": -32601, "message": format!("fake node has no route for {method}")}
        }),
    };

    let payload = serde_json::to_vec(&body).unwrap_or_default();
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(&payload).await?;
    socket.flush().await
}

#[cfg(test)]
mod tests {
    use super::FakeNode;
    use crate::rpc::RpcClient;
    use serde_json::json;
    use std::time::Duration;

    #[tokio::test]
    async fn serves_a_canned_result_and_records_the_call() {
        let node = FakeNode::builder()
            .respond("local_node_info", json!({"version": "0.5.5"}))
            .start()
            .await;
        let client = RpcClient::new(node.url(), Duration::from_secs(5)).expect("client");
        let info: serde_json::Value = client
            .call("local_node_info", json!([]))
            .await
            .expect("call succeeds");
        assert_eq!(info["version"], "0.5.5");
        assert_eq!(node.call_count("local_node_info"), 1);
    }

    #[tokio::test]
    async fn serves_a_sequence_then_repeats_the_last() {
        let node = FakeNode::builder()
            .respond_sequence("get_cells", vec![json!({"page": 1}), json!({"page": 2})])
            .start()
            .await;
        let client = RpcClient::new(node.url(), Duration::from_secs(5)).expect("client");
        let one: serde_json::Value = client.call("get_cells", json!([])).await.expect("1");
        let two: serde_json::Value = client.call("get_cells", json!([])).await.expect("2");
        let three: serde_json::Value = client.call("get_cells", json!([])).await.expect("3");
        assert_eq!(one["page"], 1);
        assert_eq!(two["page"], 2);
        assert_eq!(three["page"], 2, "the last reply repeats");
    }

    #[tokio::test]
    async fn surfaces_rpc_errors_and_unknown_methods() {
        let node = FakeNode::builder()
            .fail("get_cells", -32000, "indexer not enabled")
            .start()
            .await;
        let client = RpcClient::new(node.url(), Duration::from_secs(5)).expect("client");
        let err = client
            .call::<serde_json::Value>("get_cells", json!([]))
            .await
            .expect_err("must be an error");
        assert!(
            matches!(err, crate::BackendError::Rpc { code: -32000, .. }),
            "{err:?}"
        );

        let unknown = client
            .call::<serde_json::Value>("no_such_method", json!([]))
            .await
            .expect_err("unrouted methods error");
        assert!(
            matches!(unknown, crate::BackendError::Rpc { code: -32601, .. }),
            "{unknown:?}"
        );
    }
}
