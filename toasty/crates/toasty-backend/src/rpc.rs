//! A JSON-RPC 2.0 connection over a pair of byte streams, one JSON object per line.
//!
//! ACP is symmetric: the client sends requests to the agent, and the agent sends its own
//! requests (approvals, questions) back. [`Connection`] does both. Replies to our requests
//! are matched by id inside the reader task and completed by the event pump after preceding
//! updates. This preserves text-before-turn-completion ordering without waiting for clients.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

/// An error object returned by the other side, as defined by JSON-RPC.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message} (code {code})")]
pub struct RemoteError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RemoteError {
    /// The standard "method not found" error.
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("method not supported by Toasty: {method}"),
            data: None,
        }
    }

    /// The standard "invalid params" error.
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            data: None,
        }
    }
}

/// Why a call on the connection failed.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RpcError {
    #[error("the agent input queue is full; retry after it drains")]
    Busy,
    #[error("the agent closed the connection")]
    Closed,
    #[error("the agent answered {method} with an error: {source}")]
    Remote {
        method: String,
        #[source]
        source: Box<RemoteError>,
    },
}

/// A message the other side started: a request that needs a reply, or a notification.
#[derive(Debug)]
pub enum Incoming {
    /// Resolve transport failures after preceding updates have been consumed.
    Disconnected {
        pending: Vec<oneshot::Sender<Result<Value, RemoteError>>>,
        ordered: bool,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// Resolve a call after preceding notifications have reached the event pump.
    Response {
        completion: oneshot::Sender<Result<Value, RemoteError>>,
        outcome: Result<Value, RemoteError>,
        ordered: bool,
    },
}

type Pending = HashMap<u64, (oneshot::Sender<Result<Value, RemoteError>>, bool)>;
const QUEUE_CAPACITY: usize = 64;
const MAX_FRAME_BYTES: u64 = 1024 * 1024;

struct Shared {
    out: mpsc::Sender<String>,
    pending: Mutex<Pending>,
    next_id: AtomicU64,
}

impl Shared {
    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct PendingCall {
    shared: Arc<Shared>,
    id: u64,
}
impl Drop for PendingCall {
    fn drop(&mut self) {
        self.shared.pending().remove(&self.id);
    }
}

/// One end of a JSON-RPC connection. Cheap to clone.
#[derive(Clone)]
pub struct Connection {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Connection")
    }
}

impl Connection {
    /// Starts the reader and writer tasks. Returns the connection and the stream of messages
    /// the other side starts. The stream ends when the other side closes its output.
    pub fn start<R, W>(read: R, write: W) -> (Self, mpsc::Receiver<Incoming>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (out_tx, out_rx) = mpsc::channel(QUEUE_CAPACITY);
        let (in_tx, in_rx) = mpsc::channel(QUEUE_CAPACITY);
        let shared = Arc::new(Shared {
            out: out_tx,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        });
        tokio::spawn(write_lines(write, out_rx));
        tokio::spawn(read_lines(read, Arc::clone(&shared), in_tx));
        (Self { shared }, in_rx)
    }

    /// Sends a request and waits for its reply, however long the other side takes.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.shared.pending();
            if pending.len() >= QUEUE_CAPACITY {
                return Err(RpcError::Busy);
            }
            pending.insert(id, (tx, method == "session/prompt"));
        }
        let _pending = PendingCall {
            shared: self.shared.clone(),
            id,
        };
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if let Err(error) = self.send(&frame) {
            self.shared.pending().remove(&id);
            return Err(error);
        }
        match rx.await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(source)) => Err(RpcError::Remote {
                method: method.to_owned(),
                source: Box::new(source),
            }),
            Err(_) => Err(RpcError::Closed),
        }
    }

    /// Sends a notification, which has no reply.
    pub fn notify(&self, method: &str, params: Value) -> Result<(), RpcError> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    /// Replies to a request the other side sent.
    pub fn respond(&self, id: Value, result: Result<Value, RemoteError>) -> Result<(), RpcError> {
        let frame = match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(e) => {
                let mut error = Map::new();
                error.insert("code".into(), e.code.into());
                error.insert("message".into(), e.message.into());
                if let Some(data) = e.data {
                    error.insert("data".into(), data);
                }
                json!({"jsonrpc": "2.0", "id": id, "error": error})
            }
        };
        self.send(&frame)
    }

    fn send(&self, frame: &Value) -> Result<(), RpcError> {
        self.shared
            .out
            .try_send(frame.to_string())
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => RpcError::Busy,
                mpsc::error::TrySendError::Closed(_) => RpcError::Closed,
            })
    }
}

async fn write_lines<W: AsyncWrite + Unpin>(mut write: W, mut lines: mpsc::Receiver<String>) {
    while let Some(mut line) = lines.recv().await {
        line.push('\n');
        if let Err(e) = write.write_all(line.as_bytes()).await {
            tracing::debug!(error = %e, "agent input closed");
            return;
        }
        if let Err(e) = write.flush().await {
            tracing::debug!(error = %e, "agent input closed");
            return;
        }
    }
    // Every sender is gone: closing our end tells the agent we are done.
    let _ = write.shutdown().await;
}

async fn read_lines<R: AsyncRead + Unpin>(
    read: R,
    shared: Arc<Shared>,
    incoming: mpsc::Sender<Incoming>,
) {
    let mut reader = BufReader::new(read);
    loop {
        let mut bytes = Vec::new();
        match (&mut reader)
            .take(MAX_FRAME_BYTES + 1)
            .read_until(b'\n', &mut bytes)
            .await
        {
            Ok(0) => break,
            Ok(_) if bytes.len() as u64 > MAX_FRAME_BYTES => {
                tracing::warn!("agent frame exceeds one MiB; closing connection");
                break;
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, "reading from agent failed");
                break;
            }
        }
        let line = match std::str::from_utf8(&bytes) {
            Ok(line) => line,
            Err(error) => {
                tracing::warn!(%error, "agent sent invalid UTF-8");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let frame: Value = match serde_json::from_str(line) {
            Ok(frame) => frame,
            Err(e) => {
                tracing::warn!(error = %e, "agent sent a line that is not JSON");
                continue;
            }
        };
        if !route(frame, &shared, &incoming).await {
            break;
        }
    }
    // Failure follows the same ordering boundary as a successful turn reply.
    let (pending, ordered) = {
        let mut calls = shared.pending();
        let ordered = calls.values().any(|(_, ordered)| *ordered);
        (calls.drain().map(|(_, (tx, _))| tx).collect(), ordered)
    };
    let _ = incoming
        .send(Incoming::Disconnected { pending, ordered })
        .await;
}

async fn route(frame: Value, shared: &Shared, incoming: &mpsc::Sender<Incoming>) -> bool {
    let Value::Object(mut frame) = frame else {
        tracing::warn!("agent sent a JSON value that is not an object");
        return true;
    };
    let params = frame.remove("params").unwrap_or(Value::Null);
    match (frame.remove("method"), frame.remove("id")) {
        (Some(Value::String(method)), Some(id)) => {
            return incoming
                .send(Incoming::Request { id, method, params })
                .await
                .is_ok();
        }
        (Some(Value::String(method)), None) => {
            return incoming
                .send(Incoming::Notification { method, params })
                .await
                .is_ok();
        }
        (None, Some(id)) => {
            let Some(id) = id.as_u64() else {
                tracing::warn!(%id, "agent replied with an id we never sent");
                return true;
            };
            let Some((waiter, ordered)) = shared.pending().remove(&id) else {
                tracing::warn!(id, "agent replied to a request that is not pending");
                return true;
            };
            let outcome = match frame.remove("error") {
                Some(error) => Err(remote_error(error)),
                None => Ok(frame.remove("result").unwrap_or(Value::Null)),
            };
            return incoming
                .send(Incoming::Response {
                    completion: waiter,
                    outcome,
                    ordered,
                })
                .await
                .is_ok();
        }
        _ => tracing::warn!("agent sent a frame that is neither a request nor a reply"),
    }
    true
}

fn remote_error(error: Value) -> RemoteError {
    RemoteError {
        code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .to_owned(),
        data: error.get("data").cloned(),
    }
}
