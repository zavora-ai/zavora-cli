//! LSP JSON-RPC client with Content-Length framing over stdio.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{Mutex, oneshot};

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: i64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Serialize)]
struct JsonRpcNotification {
    jsonrpc: &'static str,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    id: Option<i64>,
    result: Option<Value>,
    error: Option<Value>,
}

pub struct LspClient {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>,
    next_id: AtomicI64,
}

impl LspClient {
    /// Create a new LSP client and spawn the stdout reader task.
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let pending_clone = pending.clone();
        tokio::spawn(async move {
            if let Err(e) = read_loop(stdout, pending_clone).await {
                tracing::debug!("LSP stdout reader ended: {}", e);
            }
        });

        Self {
            stdin: Arc::new(Mutex::new(stdin)),
            pending,
            next_id: AtomicI64::new(1),
        }
    }

    /// Send a request and wait for the response.
    pub async fn request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let body = serde_json::to_string(&msg)?;
        let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);

        self.stdin.lock().await.write_all(frame.as_bytes()).await?;

        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(anyhow::anyhow!("LSP response channel closed")),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(anyhow::anyhow!("LSP request timed out: {}", method))
            }
        }
    }

    /// Send a notification (no response expected).
    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<()> {
        let msg = JsonRpcNotification {
            jsonrpc: "2.0",
            method: method.to_string(),
            params,
        };
        let body = serde_json::to_string(&msg)?;
        let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        self.stdin.lock().await.write_all(frame.as_bytes()).await?;
        Ok(())
    }
}

/// Read Content-Length framed JSON-RPC messages from stdout and dispatch responses.
async fn read_loop(
    stdout: ChildStdout,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>,
) -> Result<()> {
    let mut reader = BufReader::new(stdout);
    let mut header_buf = String::new();

    loop {
        // Read headers until empty line
        let mut content_length: usize = 0;
        loop {
            header_buf.clear();
            let n = reader.read_line(&mut header_buf).await?;
            if n == 0 {
                return Ok(()); // EOF
            }
            let line = header_buf.trim();
            if line.is_empty() {
                break;
            }
            if let Some(len_str) = line.strip_prefix("Content-Length:") {
                content_length = len_str.trim().parse().unwrap_or(0);
            }
        }

        if content_length == 0 {
            continue;
        }

        // Read body
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).await?;

        let response: JsonRpcResponse = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(_) => continue, // Skip non-response messages (notifications from server)
        };

        if let Some(id) = response.id {
            let mut map = pending.lock().await;
            if let Some(tx) = map.remove(&id) {
                let result = if let Some(err) = response.error {
                    Err(anyhow::anyhow!("LSP error: {}", err))
                } else {
                    Ok(response.result.unwrap_or(Value::Null))
                };
                let _ = tx.send(result);
            }
        }
    }
}
