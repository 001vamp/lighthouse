//! What an agent session tells the daemon, independent of the wire protocol underneath.

use serde_json::{Map, Value, json};

use crate::rpc::{Connection, RemoteError};

/// Who wrote a piece of text in a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Agent,
    /// The agent's visible reasoning.
    Thought,
}

/// The state of a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl ToolStatus {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Why a turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    /// A reason this version of Toasty does not know yet.
    Other(String),
}

impl StopReason {
    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "end_turn" => Self::EndTurn,
            "max_tokens" => Self::MaxTokens,
            "max_turn_requests" => Self::MaxTurnRequests,
            "refusal" => Self::Refusal,
            "cancelled" => Self::Cancelled,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// Something that happened in a session.
#[derive(Debug)]
pub enum AgentEvent {
    /// Acknowledge after all preceding events have been consumed durably.
    Flush(tokio::sync::oneshot::Sender<()>),
    /// A piece of text. Pieces with the same `message_id` belong to one message. When a
    /// session is loaded, its history arrives this way too.
    Text {
        role: Role,
        message_id: Option<String>,
        text: String,
    },
    ToolCall {
        id: String,
        title: String,
        kind: Option<String>,
        status: Option<ToolStatus>,
    },
    ToolCallUpdate {
        id: String,
        title: Option<String>,
        status: Option<ToolStatus>,
    },
    Title(String),
    /// Context window use, in tokens.
    Usage {
        used: u64,
        size: u64,
    },
    /// The session's settings changed (mode, model, effort and so on).
    Config(Vec<ConfigOption>),
    /// The agent asks for permission. It waits until [`Permission::answer`] is called.
    Permission(Permission),
    /// The agent asks a question. A blocking one waits until [`Question::answer`] is called.
    Question(Question),
    /// The agent no longer needs an answer to a request, for example because its turn was
    /// cancelled.
    Withdrawn {
        request: RequestKey,
    },
    /// The adapter exited. No more events follow.
    Exited,
}

/// One setting of a session, such as its mode or model, with the values it can take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    /// What the setting is about: `mode`, `model`, `thought_level` and so on.
    pub category: Option<String>,
    pub current: String,
    pub choices: Vec<ConfigChoice>,
}

/// One value a [`ConfigOption`] can take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigChoice {
    pub value: String,
    pub name: String,
    pub description: Option<String>,
}

/// Identifies one request within its session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestKey(pub String);

impl RequestKey {
    pub(crate) fn from_id(id: &Value) -> Self {
        match id {
            Value::String(s) => Self(s.clone()),
            other => Self(other.to_string()),
        }
    }
}

/// What kind of risk an option carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

impl OptionKind {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "allow_once" => Some(Self::AllowOnce),
            "allow_always" => Some(Self::AllowAlways),
            "reject_once" => Some(Self::RejectOnce),
            "reject_always" => Some(Self::RejectAlways),
            _ => None,
        }
    }
}

/// One button on a permission request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionOption {
    pub id: String,
    pub label: String,
    pub kind: OptionKind,
}

/// What the agent wants to do, when it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    Diff {
        path: String,
        old: Option<String>,
        new: String,
    },
    Command(String),
}

/// The owner's answer to a permission request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionAnswer {
    /// The id of one of the request's options.
    Selected(String),
    /// No option chosen, for example because the turn was cancelled.
    Cancelled,
}

/// A permission request, with the means to answer it.
#[derive(Debug)]
pub struct Permission {
    pub key: RequestKey,
    pub tool_call_id: Option<String>,
    pub title: String,
    pub kind: Option<String>,
    pub detail: Option<Detail>,
    pub options: Vec<PermissionOption>,
    pub(crate) reply: Reply,
}

impl Permission {
    /// Sends the answer to the agent.
    pub fn answer(mut self, answer: PermissionAnswer) {
        let outcome = match answer {
            PermissionAnswer::Selected(id) => json!({"outcome": "selected", "optionId": id}),
            PermissionAnswer::Cancelled => json!({"outcome": "cancelled"}),
        };
        self.reply.send(json!({"outcome": outcome}));
    }
}

/// One choice in a choice field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

/// What kind of value a field takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Number { integer: bool },
    Boolean,
    Choice(Vec<Choice>),
    MultiChoice(Vec<Choice>),
}

/// One field of a question form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub required: bool,
    pub kind: FieldKind,
}

/// What the agent asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Form {
    /// Fill in these fields.
    Fields(Vec<Field>),
    /// Open this link, for example to sign in to a tool server.
    Url(String),
}

/// The owner's answer to a question.
#[derive(Debug, Clone, PartialEq)]
pub enum QuestionAnswer {
    /// The values, keyed by field id. Empty for a [`Form::Url`].
    Accept(Map<String, Value>),
    Decline,
    Cancel,
}

/// A question from the agent, with the means to answer it.
#[derive(Debug)]
pub struct Question {
    pub key: RequestKey,
    pub tool_call_id: Option<String>,
    pub message: String,
    pub form: Form,
    /// False when the agent keeps working without waiting for the answer. Codex sends such
    /// questions; an answer that arrives after its turn ends is dropped by the agent.
    pub blocking: bool,
    pub(crate) reply: Reply,
}

impl Question {
    /// Sends the answer to the agent.
    pub fn answer(mut self, answer: QuestionAnswer) {
        let result = match answer {
            QuestionAnswer::Accept(content) => json!({"action": "accept", "content": content}),
            QuestionAnswer::Decline => json!({"action": "decline"}),
            QuestionAnswer::Cancel => json!({"action": "cancel"}),
        };
        self.reply.send(result);
    }
}

/// The reply half of one agent request. If it is dropped unanswered, which only happens when
/// the session is being torn down, the agent is told the request was cancelled, so it never
/// waits on a reply that cannot come.
#[derive(Debug)]
pub(crate) struct Reply {
    connection: Option<Connection>,
    id: Value,
    on_drop: Value,
}

impl Reply {
    pub(crate) fn new(connection: Connection, id: Value, on_drop: Value) -> Self {
        Self {
            connection: Some(connection),
            id,
            on_drop,
        }
    }

    fn send(&mut self, result: Value) {
        if let Some(connection) = self.connection.take() {
            let _ = connection.respond(self.id.clone(), Ok(result));
        }
    }

    pub(crate) fn reject(mut self, error: RemoteError) {
        if let Some(connection) = self.connection.take() {
            let _ = connection.respond(self.id.clone(), Err(error));
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        let fallback = std::mem::take(&mut self.on_drop);
        self.send(fallback);
    }
}
