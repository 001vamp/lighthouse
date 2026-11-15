//! Approvals and questions shared by all daemon clients.
//!
//! Requests have no deadline. Dropping an answer future does not close its request.
//! Subscribe before taking a snapshot with [`Broker::list_open`]. A subscriber that
//! lags behind the broadcast buffer must take a fresh snapshot.

#![deny(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, broadcast, oneshot};

/// A permission option's meaning, independent of its displayed label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOptionKind {
    /// Permit this invocation.
    AllowOnce,
    /// Permit this and matching future invocations.
    AllowAlways,
    /// Reject this invocation.
    RejectOnce,
    /// Reject this and matching future invocations.
    RejectAlways,
}

/// An option supplied by the agent, with an opaque identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOption {
    /// Identifier returned to the agent when selected.
    pub id: String,
    /// Text shown to the user.
    pub label: String,
    /// The effect of selecting this option.
    pub kind: PermissionOptionKind,
}

/// A tool invocation awaiting permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permission {
    /// Tool title shown to the user.
    pub title: String,
    /// Agent-provided tool category, such as edit or execute.
    pub kind: String,
    /// Proposed changes, when supplied by the agent.
    pub diff: Option<String>,
    /// Proposed command, when supplied by the agent.
    pub command: Option<String>,
    /// Available decisions. Identifiers must be unique and nonempty.
    pub options: Vec<PermissionOption>,
}

/// A selectable value in a question field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    /// Value returned to the agent.
    pub value: String,
    /// Text shown to the user.
    pub label: String,
    /// Further explanation, if present in the form.
    pub description: Option<String>,
}

/// A typed form field. Values are checked before a request can close.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionField {
    /// Unique field identifier in an answer.
    pub id: String,
    /// Display label.
    pub title: String,
    /// Optional explanation.
    pub description: Option<String>,
    /// Whether an accepted form must contain a value.
    pub required: bool,
    /// Allowed value type.
    pub kind: FieldKind,
}

/// Supported question value types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "choices", rename_all = "snake_case")]
pub enum FieldKind {
    /// Text value.
    Text,
    /// JSON number.
    Number,
    /// Whole JSON number.
    Integer,
    /// Yes or no.
    Boolean,
    /// One supplied value.
    Choice(Vec<Choice>),
    /// Distinct supplied values.
    MultiChoice(Vec<Choice>),
}

/// A form or a link supplied by the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "content", rename_all = "snake_case")]
pub enum QuestionForm {
    /// Named, typed fields.
    Fields(Vec<QuestionField>),
    /// Link to open, for example for authentication.
    Url(String),
}

/// An agent question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// What the agent asks.
    pub message: String,
    /// What the user should fill in or open.
    pub form: QuestionForm,
    /// Whether the agent waits for the answer.
    pub blocking: bool,
}

/// A request's typed contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RequestPayload {
    /// Permission for a tool invocation.
    Permission(Permission),
    /// A blocking or asynchronous question.
    Question(Question),
}

/// A request identified across clients and scoped to a Toasty thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// Globally unique identifier. Never reuse it during a broker's lifetime.
    pub id: String,
    /// Toasty thread that owns the request.
    pub thread_id: String,
    /// Contents to display and validate against.
    pub payload: RequestPayload,
}

/// A decision about a permission request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "option_id", rename_all = "snake_case")]
pub enum PermissionAnswer {
    /// Select one of the agent's option identifiers.
    Selected(String),
    /// Explicit cancellation; dismissing a client view does not send this.
    Cancelled,
}

/// A response to a question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "values", rename_all = "snake_case")]
pub enum QuestionAnswer {
    /// Submit typed JSON values keyed by field identifier.
    Accept(BTreeMap<String, serde_json::Value>),
    /// Explicitly decline to provide an answer.
    Decline,
    /// Explicitly cancel the question.
    Cancel,
}

/// An answer must match the kind of request it resolves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "answer", rename_all = "snake_case")]
pub enum Answer {
    /// A tool permission decision.
    Permission(PermissionAnswer),
    /// A question response.
    Question(QuestionAnswer),
}

/// A state change sent to every subscribed client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A request became available to answer.
    Opened {
        /// The full request to display.
        request: Request,
    },
    /// The first valid answer closed a request.
    Answered {
        /// Identifier of the closed request.
        request_id: String,
        /// The winning answer.
        answer: Answer,
    },
    /// The agent removed a request without a user answer.
    Withdrawn {
        /// Identifier of the removed request.
        request_id: String,
    },
}

/// A failure to open or settle a request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BrokerError {
    /// The request model is malformed.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// Identifiers may not be reused, including after a request closes.
    #[error("request {0} already exists")]
    DuplicateRequest(String),
    /// No request has ever used this identifier.
    #[error("unknown request {0}")]
    UnknownRequest(String),
    /// Another answer already won.
    #[error("request {0} was already answered")]
    AlreadyAnswered(String),
    /// The agent withdrew the request.
    #[error("request {0} was withdrawn")]
    Withdrawn(String),
    /// The answer variant does not match the request variant.
    #[error("answer kind does not match request")]
    WrongAnswerKind,
    /// A selected permission option does not exist.
    #[error("unknown permission option {0}")]
    UnknownOption(String),
    /// An accepted form omits a required field.
    #[error("required field {0} is missing")]
    MissingRequiredField(String),
    /// An accepted form includes a field not in the request.
    #[error("unknown question field {0}")]
    UnknownField(String),
    /// A choice answer is not one of the supplied values.
    #[error("invalid choice {value:?} for field {field}")]
    InvalidChoice {
        /// Field receiving the invalid value.
        field: String,
        /// Value rejected by the form.
        value: String,
    },
    /// A field value has the wrong JSON type or contains duplicate selections.
    #[error("invalid value for field {0}")]
    InvalidValue(String),
    /// All broker handles were dropped before the request was settled.
    #[error("broker stopped before the request was settled")]
    Closed,
}

/// A future for the winning answer, or withdrawal. It has no timeout.
#[must_use = "await this future to receive the agent's answer"]
pub struct PendingAnswer {
    receiver: oneshot::Receiver<Result<Answer, BrokerError>>,
}

impl Future for PendingAnswer {
    type Output = Result<Answer, BrokerError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.receiver).poll(cx) {
            Poll::Ready(Ok(result)) => Poll::Ready(result),
            Poll::Ready(Err(_)) => Poll::Ready(Err(BrokerError::Closed)),
            Poll::Pending => Poll::Pending,
        }
    }
}

enum State {
    Open {
        request: Request,
        reply: oneshot::Sender<Result<Answer, BrokerError>>,
    },
    Answered,
    Withdrawn,
}

/// A cloneable broker. One lock serializes validation, settlement and events.
///
/// Closed identifiers are retained as tombstones so later answers can never win.
/// Requests and answers are released after settlement. Persistence belongs to the
/// daemon: save a request before opening it and save an answer before forwarding
/// it to the agent. Broadcast events are notifications, not a durable journal.
#[derive(Clone)]
pub struct Broker {
    requests: Arc<Mutex<BTreeMap<String, State>>>,
    events: broadcast::Sender<Event>,
}

impl Default for Broker {
    fn default() -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            requests: Arc::new(Mutex::new(BTreeMap::new())),
            events,
        }
    }
}

impl Broker {
    /// Create a broker with the default event buffer of 256 changes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to future changes. On lag, call [`Self::list_open`] again.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Validate and open a request, returning its indefinitely waiting future.
    ///
    /// Dropping the future does not answer or withdraw the request. Duplicate
    /// identifiers are rejected even if the earlier request has already closed.
    pub async fn open(&self, request: Request) -> Result<PendingAnswer, BrokerError> {
        validate_request(&request)?;
        let mut requests = self.requests.lock().await;
        if requests.contains_key(&request.id) {
            return Err(BrokerError::DuplicateRequest(request.id));
        }
        let (reply, receiver) = oneshot::channel();
        requests.insert(
            request.id.clone(),
            State::Open {
                request: request.clone(),
                reply,
            },
        );
        let _ = self.events.send(Event::Opened { request });
        Ok(PendingAnswer { receiver })
    }

    /// Settle an open request with the first valid answer.
    ///
    /// Invalid answers leave the request open and emit no event. No await occurs
    /// between validation and settlement, including when two clients race.
    pub async fn answer(&self, request_id: &str, answer: Answer) -> Result<(), BrokerError> {
        let mut requests = self.requests.lock().await;
        let state = requests
            .get_mut(request_id)
            .ok_or_else(|| BrokerError::UnknownRequest(request_id.into()))?;
        match state {
            State::Open { request, .. } => validate_answer(&request.payload, &answer)?,
            State::Answered => return Err(BrokerError::AlreadyAnswered(request_id.into())),
            State::Withdrawn => return Err(BrokerError::Withdrawn(request_id.into())),
        }
        if let State::Open { reply, .. } = std::mem::replace(state, State::Answered) {
            let _ = self.events.send(Event::Answered {
                request_id: request_id.into(),
                answer: answer.clone(),
            });
            let _ = reply.send(Ok(answer));
        }
        Ok(())
    }

    /// Remove an open request on behalf of its agent and wake its waiter.
    pub async fn withdraw(&self, request_id: &str) -> Result<(), BrokerError> {
        let mut requests = self.requests.lock().await;
        let state = requests
            .get_mut(request_id)
            .ok_or_else(|| BrokerError::UnknownRequest(request_id.into()))?;
        match state {
            State::Answered => return Err(BrokerError::AlreadyAnswered(request_id.into())),
            State::Withdrawn => return Err(BrokerError::Withdrawn(request_id.into())),
            State::Open { .. } => {}
        }
        if let State::Open { reply, .. } = std::mem::replace(state, State::Withdrawn) {
            let _ = self.events.send(Event::Withdrawn {
                request_id: request_id.into(),
            });
            let _ = reply.send(Err(BrokerError::Withdrawn(request_id.into())));
        }
        Ok(())
    }

    /// Return a snapshot ordered by request identifier.
    pub async fn list_open(&self) -> Vec<Request> {
        self.requests
            .lock()
            .await
            .values()
            .filter_map(|state| match state {
                State::Open { request, .. } => Some(request.clone()),
                State::Answered | State::Withdrawn => None,
            })
            .collect()
    }
}

fn nonempty(value: &str, name: &str) -> Result<(), BrokerError> {
    if value.trim().is_empty() {
        return Err(BrokerError::InvalidRequest(format!("{name} is empty")));
    }
    Ok(())
}

/// Check an incoming request without changing broker state.
pub fn validate_request(request: &Request) -> Result<(), BrokerError> {
    nonempty(&request.id, "request id")?;
    nonempty(&request.thread_id, "thread id")?;
    match &request.payload {
        RequestPayload::Permission(permission) => {
            nonempty(&permission.title, "tool title")?;
            nonempty(&permission.kind, "tool kind")?;
            if permission.options.is_empty() {
                return Err(BrokerError::InvalidRequest(
                    "permission has no options".into(),
                ));
            }
            let mut ids = BTreeSet::new();
            for option in &permission.options {
                nonempty(&option.id, "option id")?;
                nonempty(&option.label, "option label")?;
                if !ids.insert(&option.id) {
                    return Err(BrokerError::InvalidRequest(format!(
                        "duplicate option {}",
                        option.id
                    )));
                }
            }
        }
        RequestPayload::Question(question) => {
            nonempty(&question.message, "question message")?;
            let QuestionForm::Fields(fields) = &question.form else {
                if let QuestionForm::Url(url) = &question.form {
                    nonempty(url, "question URL")?;
                }
                return Ok(());
            };
            let mut ids = BTreeSet::new();
            for field in fields {
                nonempty(&field.id, "field id")?;
                nonempty(&field.title, "field title")?;
                if !ids.insert(&field.id) {
                    return Err(BrokerError::InvalidRequest(format!(
                        "duplicate field {}",
                        field.id
                    )));
                }
                if let FieldKind::Choice(choices) | FieldKind::MultiChoice(choices) = &field.kind {
                    if choices.is_empty() {
                        return Err(BrokerError::InvalidRequest(format!(
                            "field {} has no choices",
                            field.id
                        )));
                    }
                    let mut values = BTreeSet::new();
                    for choice in choices {
                        nonempty(&choice.label, "choice label")?;
                        if !values.insert(&choice.value) {
                            return Err(BrokerError::InvalidRequest(format!(
                                "duplicate choice in field {}",
                                field.id
                            )));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Check an answer without settling the request.
pub fn validate_answer(payload: &RequestPayload, answer: &Answer) -> Result<(), BrokerError> {
    match (payload, answer) {
        (RequestPayload::Permission(permission), Answer::Permission(answer)) => {
            if let PermissionAnswer::Selected(id) = answer
                && !permission.options.iter().any(|option| option.id == *id)
            {
                return Err(BrokerError::UnknownOption(id.clone()));
            }
        }
        (RequestPayload::Question(question), Answer::Question(QuestionAnswer::Accept(values))) => {
            let fields = match &question.form {
                QuestionForm::Fields(fields) => fields.as_slice(),
                QuestionForm::Url(_) => &[],
            };
            for id in values.keys() {
                if !fields.iter().any(|field| &field.id == id) {
                    return Err(BrokerError::UnknownField(id.clone()));
                }
            }
            for field in fields {
                let Some(value) = values.get(&field.id) else {
                    if field.required {
                        return Err(BrokerError::MissingRequiredField(field.id.clone()));
                    }
                    continue;
                };
                let valid = match &field.kind {
                    FieldKind::Text => value.is_string(),
                    FieldKind::Number => value.is_number(),
                    FieldKind::Integer => {
                        value.as_i64().is_some()
                            || value.as_u64().is_some()
                            || value.as_f64().is_some_and(|n| n.fract() == 0.0)
                    }
                    FieldKind::Boolean => value.is_boolean(),
                    FieldKind::Choice(choices) => value
                        .as_str()
                        .is_some_and(|v| choices.iter().any(|c| c.value == v)),
                    FieldKind::MultiChoice(choices) => value.as_array().is_some_and(|selected| {
                        let mut seen = BTreeSet::new();
                        selected.iter().all(|v| {
                            v.as_str().is_some_and(|v| {
                                seen.insert(v) && choices.iter().any(|c| c.value == v)
                            })
                        })
                    }),
                };
                if !valid {
                    return Err(BrokerError::InvalidValue(field.id.clone()));
                }
            }
        }
        (RequestPayload::Question(_), Answer::Question(_)) => {}
        _ => return Err(BrokerError::WrongAnswerKind),
    }
    Ok(())
}
