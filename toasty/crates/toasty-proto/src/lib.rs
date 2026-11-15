//! Versioned, newline-delimited JSON messages for the owner-only daemon socket.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub use toasty_broker::{Answer, RequestPayload};
pub use toasty_store::{AgentKind, Message, MessagePage, StoredRequest, Thread, ThreadId};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub protocol: u32,
    pub client: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMessage {
    pub id: u64,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Run {
        agent: AgentKind,
        cwd: PathBuf,
        prompt: String,
        #[serde(default)]
        config: Vec<(String, String)>,
    },
    List,
    Messages {
        thread_id: ThreadId,
        #[serde(default)]
        after: i64,
        #[serde(default = "default_message_limit")]
        limit: u32,
    },
    Requests {
        thread_id: Option<ThreadId>,
    },
    Reply {
        thread_id: ThreadId,
        text: String,
    },
    Steer {
        thread_id: ThreadId,
        text: String,
    },
    Cancel {
        thread_id: ThreadId,
    },
    Answer {
        thread_id: ThreadId,
        request_id: String,
        answer: Answer,
    },
    Resume {
        thread_id: ThreadId,
    },
    Close {
        thread_id: ThreadId,
    },
    Subscribe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Reply {
    Thread(Thread),
    Threads(Vec<Thread>),
    Messages(MessagePage),
    Requests(Vec<StoredRequest>),
    Ok,
    Subscribed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    NotFound,
    Conflict,
    Agent,
    Storage,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Failure {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Event {
    ThreadChanged(Thread),
    Message(Message),
    RequestChanged(StoredRequest),
    Tool {
        thread_id: ThreadId,
        id: String,
        title: Option<String>,
        kind: Option<String>,
        status: Option<String>,
    },
    Usage {
        thread_id: ThreadId,
        used: u64,
        size: u64,
    },
    Error {
        thread_id: ThreadId,
        error: Failure,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Hello {
        protocol: u32,
    },
    Response {
        id: u64,
        result: Result<Reply, Failure>,
    },
    Event {
        event: Event,
    },
    ResyncRequired,
    Error {
        error: Failure,
    },
}

fn default_message_limit() -> u32 {
    100
}
