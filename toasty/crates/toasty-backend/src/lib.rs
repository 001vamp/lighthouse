//! Agent backends for toastyd.
//!
//! Every agent sits behind the same two traits: a [`Backend`] opens sessions, and a
//! [`Session`] takes prompts and reports [`AgentEvent`]s. Today the only backend is ACP,
//! which drives both Claude Code and Codex through their pinned adapters. A native backend
//! can replace it for one agent without the daemon noticing.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;

mod acp;
pub mod adapters;
pub mod events;
pub mod process;
pub mod rpc;

pub use acp::AcpBackend;
pub use events::*;

/// A boxed future, so the traits stay usable as `dyn` objects.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Which agent a session runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    Codex,
}

impl AgentKind {
    /// Every agent Toasty can start.
    pub const ALL: [Self; 2] = [Self::Claude, Self::Codex];

    /// The stable name used in storage and on the socket.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    /// The reverse of [`AgentKind::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// The agent's own id for a session. For Claude and Codex this is also the id their
/// command lines take to resume it in a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// How to open a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// A new, empty session.
    New,
    /// Reopen a session, falling back to loading when the agent cannot resume.
    /// Replayed history is suppressed because the durable store owns scrollback.
    Resume(SessionId),
}

/// Everything needed to open a session.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    pub agent: AgentKind,
    pub cwd: PathBuf,
    pub start: Start,
    /// Settings to apply before the first prompt, as (option id, value) pairs. See
    /// [`Session::config`] for the ids an agent offers, such as `mode` and `model`.
    pub config: Vec<(String, String)>,
}

/// Why a session could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error(transparent)]
    Adapter(#[from] adapters::AdapterError),
    #[error(transparent)]
    Spawn(#[from] process::SpawnError),
    #[error("the {agent} adapter failed while opening the session: {source}")]
    Protocol {
        agent: &'static str,
        #[source]
        source: SessionError,
    },
}

/// Why a call on a session failed.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Rpc(#[from] rpc::RpcError),
    #[error("the agent sent a reply Toasty does not understand: {0}")]
    BadReply(String),
    #[error("this agent does not support {0}")]
    Unsupported(&'static str),
    #[error("this agent has no setting `{id}` with the value `{value}`")]
    UnknownSetting { id: String, value: String },
}

/// What happened to a message sent with [`Session::steer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteerOutcome {
    /// The message joined the running turn.
    Injected,
    /// No turn was running, so nothing was sent. Send it with [`Session::prompt`] instead.
    NoRunningTurn,
}

/// A session that has been opened.
pub struct Opened {
    pub session: Arc<dyn Session>,
    /// Everything the agent reports. Ends with [`AgentEvent::Exited`].
    pub events: mpsc::Receiver<AgentEvent>,
}

/// Opens agent sessions.
pub trait Backend: Send + Sync {
    /// Starts the agent and opens a session. The session id is known as soon as this
    /// returns, before any prompt is sent, so the caller can save it first.
    fn open(&self, options: SessionOptions) -> BoxFuture<'_, Result<Opened, OpenError>>;
}

/// One open agent session.
pub trait Session: Send + Sync {
    fn id(&self) -> &SessionId;
    fn agent(&self) -> AgentKind;

    /// The session's current settings.
    fn config(&self) -> Vec<ConfigOption>;

    /// Changes one setting, such as `mode` or `model`.
    fn set_config<'a>(
        &'a self,
        id: &'a str,
        value: &'a str,
    ) -> BoxFuture<'a, Result<(), SessionError>>;

    /// Sends a prompt and waits for the turn to end, however long it takes. Events stream
    /// meanwhile. A prompt sent while a turn runs waits for that turn to end; use
    /// [`Session::steer`] to add to the running turn instead.
    fn prompt(&self, text: String) -> BoxFuture<'_, Result<StopReason, SessionError>>;

    /// Adds a message to the running turn.
    fn steer(&self, text: String) -> BoxFuture<'_, Result<SteerOutcome, SessionError>>;

    /// Asks the agent to stop the running turn. The pending [`Session::prompt`] then ends
    /// with [`StopReason::Cancelled`].
    fn cancel(&self) -> Result<(), SessionError>;

    /// Ends the session and stops the adapter and everything it started.
    fn close(&self) -> BoxFuture<'_, ()>;
}
