use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::StoreError;

macro_rules! identifier {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validate a nonempty identifier without changing its contents.
            pub fn new(value: impl Into<String>) -> Result<Self, StoreError> {
                let value = value.into();
                if value.trim().is_empty() || value.contains('\0') {
                    return Err(StoreError::InvalidInput(
                        concat!(stringify!($name), " must be nonempty and contain no NUL").into(),
                    ));
                }
                Ok(Self(value))
            }

            /// Borrow the identifier's text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = StoreError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

identifier!(
    ThreadId,
    "A nonempty Toasty thread identifier, distinct from an agent session ID."
);
identifier!(
    SessionId,
    "A nonempty agent session identifier, saved before the first prompt."
);
identifier!(
    RequestId,
    "A nonempty identifier for a persisted approval or question."
);

/// The runtime responsible for a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    /// Claude Code.
    Claude,
    /// Codex.
    Codex,
}

impl AgentKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

/// A thread's current lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    /// Ready for another prompt.
    Idle,
    /// A turn is executing.
    Running,
    /// A turn is waiting for user input.
    Waiting,
    /// Execution stopped before the turn completed.
    Interrupted,
    /// Execution failed.
    Failed,
    /// No further work is expected.
    Closed,
}

impl ThreadStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
            Self::Closed => "closed",
        }
    }
}

/// A scrollback message's author.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// A prompt or follow-up from the user.
    User,
    /// Text emitted by the agent.
    Agent,
    /// A daemon status or explanatory message.
    System,
}

impl MessageRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
            Self::System => "system",
        }
    }
}

/// Data required to persist a new, idle thread.
///
/// Create the agent session first, then await [`crate::Store::create_thread`]
/// before sending the first prompt. A thread without a session ID cannot exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewThread {
    /// Toasty's own thread identifier.
    pub id: ThreadId,
    /// Runtime that owns the agent session.
    pub agent: AgentKind,
    /// The ID returned by the agent at session creation.
    pub session_id: SessionId,
    /// Absolute working directory. SQLite storage requires a UTF-8 path.
    pub cwd: PathBuf,
    /// Settings reapplied when a session is reopened.
    pub config: Vec<(String, String)>,
    /// Human-readable thread title; it may initially be empty.
    pub title: String,
}

/// A durable thread index entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    /// Toasty's own thread identifier.
    pub id: ThreadId,
    /// Runtime that owns the session.
    pub agent: AgentKind,
    /// Agent session ID used for recovery and terminal handoff.
    pub session_id: SessionId,
    /// Working directory used to resume the agent.
    pub cwd: PathBuf,
    /// Saved settings, including agent mode and model.
    pub config: Vec<(String, String)>,
    /// Human-readable title.
    pub title: String,
    /// Current lifecycle state.
    pub status: ThreadStatus,
    /// Creation time in Unix milliseconds.
    pub created_at: i64,
    /// Last change time in Unix milliseconds.
    pub updated_at: i64,
}

/// One cached message. Sequences start at one and increase within each thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Thread containing the message.
    pub thread_id: ThreadId,
    /// Position within this thread, bounded by SQLite's signed integer range.
    pub seq: i64,
    /// Who produced the message.
    pub role: MessageRole,
    /// Message text, including partial replies preserved after interruption.
    pub text: String,
    /// Creation time in Unix milliseconds.
    pub created_at: i64,
}

/// A persisted request's state, with an answer only when it has been answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "answer", rename_all = "snake_case")]
pub enum RequestState {
    /// Awaiting an answer indefinitely.
    Open,
    /// Closed by a user answer, kept as JSON.
    Answered(Value),
    /// Removed by the agent without a user answer.
    Withdrawn,
}

/// A complete request to insert or update.
///
/// Existing request identities and payloads cannot change. Terminal states can
/// only be repeated with the same answer; they cannot be reopened or replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestInput {
    /// Stable request identifier.
    pub id: RequestId,
    /// Thread that owns the request.
    pub thread_id: ThreadId,
    /// Approval or question payload, stored as JSON text.
    pub payload: Value,
    /// Open, answered, or withdrawn.
    pub state: RequestState,
}

/// A durable request with timestamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredRequest {
    /// Identity, contents and state of the request.
    pub request: RequestInput,
    /// Creation time in Unix milliseconds.
    pub created_at: i64,
    /// Last change time in Unix milliseconds.
    pub updated_at: i64,
}

/// A bounded page of durable scrollback, with an exclusive continuation cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagePage {
    /// Text chunks in sequence order.
    pub messages: Vec<Message>,
    /// Fetch again with this `after` cursor; None means caught up at query time.
    pub next_after: Option<i64>,
}
