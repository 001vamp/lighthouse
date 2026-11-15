//! The ACP backend: drives an agent through its pinned ACP adapter over stdio.
//!
//! Method names and shapes follow what the adapters actually sent in the spike
//! (`spike/RESULTS.html`), including two extensions both adapters advertise:
//! `_session/steering` and form questions through `elicitation/create`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use crate::adapters::{AdapterSpec, Adapters};
use crate::events::{
    AgentEvent, Choice, ConfigChoice, ConfigOption, Detail, Field, FieldKind, Form, OptionKind,
    Permission, PermissionOption, Question, Reply, RequestKey, Role, StopReason, ToolStatus,
};
use crate::process::{AdapterProcess, Registry};
use crate::rpc::{Connection, Incoming, RemoteError};
use crate::{
    AgentKind, Backend, BoxFuture, OpenError, Opened, Session, SessionError, SessionId,
    SessionOptions, Start, SteerOutcome,
};

/// The ACP protocol version Toasty speaks.
const PROTOCOL_VERSION: u64 = 1;

/// How long `session/close` may take before the adapter is stopped anyway.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// Opens sessions through the pinned ACP adapters.
pub struct AcpBackend {
    adapters: Adapters,
    registry: Registry,
}

impl AcpBackend {
    /// `adapters` says where the adapters are installed; `registry` records running adapter
    /// groups so a crashed daemon's leftovers can be stopped.
    pub fn new(adapters: Adapters, registry: Registry) -> Self {
        Self { adapters, registry }
    }
}

impl Backend for AcpBackend {
    fn open(&self, options: SessionOptions) -> BoxFuture<'_, Result<Opened, OpenError>> {
        Box::pin(async move {
            let spec = AdapterSpec::for_agent(options.agent);
            let command = self.adapters.prepare(&spec, &options.cwd).await?;
            let (process, stdin, stdout) = AdapterProcess::spawn(&command, &self.registry)?;
            let (connection, incoming) = Connection::start(stdout, stdin);
            let config = Arc::new(Mutex::new(Vec::new()));
            let (events_tx, events) = mpsc::channel(256);
            let replaying = Arc::new(AtomicBool::new(matches!(options.start, Start::Resume(_))));
            tokio::spawn(pump(
                incoming,
                connection.clone(),
                events_tx,
                Arc::clone(&config),
                replaying.clone(),
            ));
            let session = handshake(options, connection, config, process, replaying)
                .await
                .map_err(|source| OpenError::Protocol {
                    agent: spec.kind.as_str(),
                    source,
                })?;
            Ok(Opened {
                session: Arc::new(session),
                events,
            })
        })
    }
}

/// What the agent said it can do.
#[derive(Debug, Clone, Copy)]
struct Caps {
    load: bool,
    resume: bool,
    close: bool,
    steering: bool,
}

async fn handshake(
    options: SessionOptions,
    connection: Connection,
    config: Arc<Mutex<Vec<ConfigOption>>>,
    process: AdapterProcess,
    replaying: Arc<AtomicBool>,
) -> Result<AcpSession, SessionError> {
    let init = connection
        .request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": {"readTextFile": false, "writeTextFile": false},
                    "terminal": false,
                    "elicitation": {"form": {}, "url": {}},
                },
                "clientInfo": {"name": "toasty", "version": env!("CARGO_PKG_VERSION")},
            }),
        )
        .await?;
    if init.get("protocolVersion").and_then(Value::as_u64) != Some(PROTOCOL_VERSION) {
        return Err(SessionError::BadReply(format!(
            "agent speaks ACP version {}, Toasty speaks {PROTOCOL_VERSION}",
            init.get("protocolVersion").unwrap_or(&Value::Null)
        )));
    }
    let agent_caps = init.get("agentCapabilities");
    let session_caps = agent_caps.and_then(|c| c.get("sessionCapabilities"));
    let caps = Caps {
        load: agent_caps
            .and_then(|c| c.get("loadSession"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        resume: session_caps.is_some_and(|c| c.get("resume").is_some()),
        close: session_caps.is_some_and(|c| c.get("close").is_some()),
        steering: init
            .pointer("/_meta/steering/supported")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };

    let cwd = options.cwd.display().to_string();
    let (method, id) = match &options.start {
        Start::New => ("session/new", None),
        Start::Resume(id) if caps.resume => ("session/resume", Some(id)),
        Start::Resume(id) if caps.load => ("session/load", Some(id)),
        Start::Resume(_) => {
            return Err(SessionError::Unsupported("reopening sessions"));
        }
    };
    let mut params = json!({"cwd": cwd, "mcpServers": []});
    if let (Some(id), Some(map)) = (id, params.as_object_mut()) {
        map.insert("sessionId".into(), id.0.clone().into());
    }
    let reply = connection.request(method, params).await?;
    replaying.store(false, Ordering::Release);
    let id = match id {
        Some(id) => id.clone(),
        None => SessionId(
            reply
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or_else(|| SessionError::BadReply("session/new returned no sessionId".into()))?
                .to_owned(),
        ),
    };
    if let Some(options) = reply.get("configOptions").and_then(parse_config_options) {
        *lock(&config) = options;
    }

    let session = AcpSession {
        id,
        agent: options.agent,
        connection,
        caps,
        config,
        process: tokio::sync::Mutex::new(Some(process)),
    };
    for (id, value) in &options.config {
        session.apply_config(id, value).await?;
    }
    Ok(session)
}

struct AcpSession {
    id: SessionId,
    agent: AgentKind,
    connection: Connection,
    caps: Caps,
    config: Arc<Mutex<Vec<ConfigOption>>>,
    process: tokio::sync::Mutex<Option<AdapterProcess>>,
}

impl AcpSession {
    async fn apply_config(&self, id: &str, value: &str) -> Result<(), SessionError> {
        let known = lock(&self.config)
            .iter()
            .any(|o| o.id == id && o.choices.iter().any(|c| c.value == value));
        if !known {
            return Err(SessionError::UnknownSetting {
                id: id.to_owned(),
                value: value.to_owned(),
            });
        }
        let reply = self
            .connection
            .request(
                "session/set_config_option",
                json!({"sessionId": self.id.0, "configId": id, "value": value}),
            )
            .await?;
        let mut config = lock(&self.config);
        match reply.get("configOptions").and_then(parse_config_options) {
            Some(options) => *config = options,
            None => {
                if let Some(option) = config.iter_mut().find(|o| o.id == id) {
                    value.clone_into(&mut option.current);
                }
            }
        }
        Ok(())
    }
}

impl Session for AcpSession {
    fn id(&self) -> &SessionId {
        &self.id
    }

    fn agent(&self) -> AgentKind {
        self.agent
    }

    fn config(&self) -> Vec<ConfigOption> {
        lock(&self.config).clone()
    }

    fn set_config<'a>(
        &'a self,
        id: &'a str,
        value: &'a str,
    ) -> BoxFuture<'a, Result<(), SessionError>> {
        Box::pin(self.apply_config(id, value))
    }

    fn prompt(&self, text: String) -> BoxFuture<'_, Result<StopReason, SessionError>> {
        Box::pin(async move {
            let reply = self
                .connection
                .request(
                    "session/prompt",
                    json!({"sessionId": self.id.0, "prompt": [{"type": "text", "text": text}]}),
                )
                .await?;
            reply
                .get("stopReason")
                .and_then(Value::as_str)
                .map(StopReason::parse)
                .ok_or_else(|| {
                    SessionError::BadReply("session/prompt returned no stopReason".into())
                })
        })
    }

    fn steer(&self, text: String) -> BoxFuture<'_, Result<SteerOutcome, SessionError>> {
        Box::pin(async move {
            if !self.caps.steering {
                return Err(SessionError::Unsupported("adding to a running turn"));
            }
            let reply = self
                .connection
                .request(
                    "_session/steering",
                    json!({
                        "sessionId": self.id.0,
                        "prompt": [{"type": "text", "text": text}],
                        // With no running turn, send nothing and say so, rather than
                        // starting a turn whose end Toasty could not follow.
                        "_meta": {"steering": {"idleBehavior": "promptRequired"}},
                    }),
                )
                .await?;
            match reply.get("outcome").and_then(Value::as_str) {
                Some("injected") => Ok(SteerOutcome::Injected),
                Some("promptRequired") => Ok(SteerOutcome::NoRunningTurn),
                other => Err(SessionError::BadReply(format!(
                    "unexpected steering outcome {other:?}"
                ))),
            }
        })
    }

    fn cancel(&self) -> Result<(), SessionError> {
        Ok(self
            .connection
            .notify("session/cancel", json!({"sessionId": self.id.0}))?)
    }

    fn close(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if self.caps.close {
                let close = self
                    .connection
                    .request("session/close", json!({"sessionId": self.id.0}));
                if tokio::time::timeout(CLOSE_TIMEOUT, close).await.is_err() {
                    tracing::warn!(session = %self.id, "session/close timed out");
                }
            }
            if let Some(process) = self.process.lock().await.take() {
                process.shutdown().await;
            }
        })
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Turns everything the adapter starts into [`AgentEvent`]s, until it exits.
async fn pump(
    mut incoming: mpsc::Receiver<Incoming>,
    connection: Connection,
    events: mpsc::Sender<AgentEvent>,
    config: Arc<Mutex<Vec<ConfigOption>>>,
    replaying: Arc<AtomicBool>,
) {
    while let Some(message) = incoming.recv().await {
        let event = match message {
            Incoming::Disconnected { pending, ordered } => {
                if ordered {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    if events.send(AgentEvent::Flush(tx)).await.is_ok() {
                        let _ = rx.await;
                    }
                }
                drop(pending);
                break;
            }
            Incoming::Response {
                completion,
                outcome,
                ordered,
            } => {
                if ordered {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    if events.send(AgentEvent::Flush(tx)).await.is_err() || rx.await.is_err() {
                        break;
                    }
                }
                let _ = completion.send(outcome);
                None
            }
            Incoming::Notification { method, params } => match method.as_str() {
                "session/update" => params.get("update").and_then(|u| parse_update(u, &config)),
                "$/cancel_request" => params.get("requestId").map(|id| AgentEvent::Withdrawn {
                    request: RequestKey::from_id(id),
                }),
                _ => None,
            },
            Incoming::Request { id, method, params } => {
                agent_request(&connection, id, &method, &params)
            }
        };
        if let Some(event) = event {
            if replaying.load(Ordering::Acquire)
                && !matches!(
                    event,
                    AgentEvent::Permission(_)
                        | AgentEvent::Question(_)
                        | AgentEvent::Withdrawn { .. }
                )
            {
                continue;
            }
            if events.send(event).await.is_err() {
                // Nobody is listening any more; the session is being torn down.
                break;
            }
        }
    }
    let _ = events.send(AgentEvent::Exited).await;
}

/// Handles a request from the agent. Approvals and questions become events; anything else
/// is refused, since Toasty offers the agent no files or terminals of its own.
fn agent_request(
    connection: &Connection,
    id: Value,
    method: &str,
    params: &Value,
) -> Option<AgentEvent> {
    let key = RequestKey::from_id(&id);
    match method {
        "session/request_permission" => {
            let reply = Reply::new(
                connection.clone(),
                id,
                json!({"outcome": {"outcome": "cancelled"}}),
            );
            match parse_permission(key, params, reply) {
                Ok(permission) => Some(AgentEvent::Permission(permission)),
                Err(rejected) => {
                    let (reply, why) = *rejected;
                    reply.reject(RemoteError::invalid_params(why));
                    None
                }
            }
        }
        "elicitation/create" => {
            let reply = Reply::new(connection.clone(), id, json!({"action": "cancel"}));
            match parse_question(key, params, reply) {
                Ok(question) => Some(AgentEvent::Question(question)),
                Err(rejected) => {
                    let (reply, why) = *rejected;
                    reply.reject(RemoteError::invalid_params(why));
                    None
                }
            }
        }
        other => {
            let _ = connection.respond(id, Err(RemoteError::method_not_found(other)));
            None
        }
    }
}

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn parse_update(update: &Value, config: &Mutex<Vec<ConfigOption>>) -> Option<AgentEvent> {
    let text = |role| {
        let content = update.get("content")?;
        (str_at(content, "type") == Some("text")).then(|| AgentEvent::Text {
            role,
            message_id: str_at(update, "messageId").map(str::to_owned),
            text: str_at(content, "text").unwrap_or_default().to_owned(),
        })
    };
    match str_at(update, "sessionUpdate")? {
        "agent_message_chunk" => text(Role::Agent),
        "user_message_chunk" => text(Role::User),
        "agent_thought_chunk" => text(Role::Thought),
        "tool_call" => Some(AgentEvent::ToolCall {
            id: str_at(update, "toolCallId")?.to_owned(),
            title: str_at(update, "title").unwrap_or_default().to_owned(),
            kind: str_at(update, "kind").map(str::to_owned),
            status: str_at(update, "status").and_then(ToolStatus::parse),
        }),
        "tool_call_update" => Some(AgentEvent::ToolCallUpdate {
            id: str_at(update, "toolCallId")?.to_owned(),
            title: str_at(update, "title").map(str::to_owned),
            status: str_at(update, "status").and_then(ToolStatus::parse),
        }),
        "session_info_update" => str_at(update, "title").map(|t| AgentEvent::Title(t.to_owned())),
        "usage_update" => Some(AgentEvent::Usage {
            used: update.get("used").and_then(Value::as_u64)?,
            size: update.get("size").and_then(Value::as_u64)?,
        }),
        "config_option_update" => {
            let options = update.get("configOptions").and_then(parse_config_options)?;
            *lock(config) = options.clone();
            Some(AgentEvent::Config(options))
        }
        // Slash commands, plans and the rest are not shown by Toasty yet.
        _ => None,
    }
}

fn parse_config_options(v: &Value) -> Option<Vec<ConfigOption>> {
    v.as_array()?
        .iter()
        .map(|o| {
            Some(ConfigOption {
                id: str_at(o, "id")?.to_owned(),
                name: str_at(o, "name").unwrap_or_default().to_owned(),
                category: str_at(o, "category").map(str::to_owned),
                current: str_at(o, "currentValue").unwrap_or_default().to_owned(),
                choices: o
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|choices| {
                        choices
                            .iter()
                            .filter_map(|c| {
                                Some(ConfigChoice {
                                    value: str_at(c, "value")?.to_owned(),
                                    name: str_at(c, "name").unwrap_or_default().to_owned(),
                                    description: str_at(c, "description").map(str::to_owned),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// A request Toasty cannot show, with the reply used to refuse it.
type Rejected = Box<(Reply, String)>;

fn parse_permission(key: RequestKey, params: &Value, reply: Reply) -> Result<Permission, Rejected> {
    let tool = params.get("toolCall").unwrap_or(&Value::Null);
    let mut options = Vec::new();
    for o in params
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(id), Some(kind)) = (
            str_at(o, "optionId"),
            str_at(o, "kind").and_then(OptionKind::parse),
        ) else {
            return Err(Box::new((
                reply,
                format!("Toasty cannot show the permission option {o}"),
            )));
        };
        options.push(PermissionOption {
            id: id.to_owned(),
            label: str_at(o, "name").unwrap_or(id).to_owned(),
            kind,
        });
    }
    if options.is_empty() {
        return Err(Box::new((
            reply,
            "permission request has no options".into(),
        )));
    }
    Ok(Permission {
        key,
        tool_call_id: str_at(tool, "toolCallId").map(str::to_owned),
        title: str_at(tool, "title")
            .unwrap_or("Permission needed")
            .to_owned(),
        kind: str_at(tool, "kind").map(str::to_owned),
        detail: parse_detail(tool),
        options,
        reply,
    })
}

fn parse_detail(tool: &Value) -> Option<Detail> {
    let diff = tool
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|c| str_at(c, "type") == Some("diff"));
    if let Some(diff) = diff {
        return Some(Detail::Diff {
            path: str_at(diff, "path")?.to_owned(),
            old: str_at(diff, "oldText").map(str::to_owned),
            new: str_at(diff, "newText").unwrap_or_default().to_owned(),
        });
    }
    match tool.pointer("/rawInput/command")? {
        Value::String(command) => Some(Detail::Command(command.clone())),
        Value::Array(parts) => Some(Detail::Command(
            parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        )),
        _ => None,
    }
}

fn parse_question(key: RequestKey, params: &Value, reply: Reply) -> Result<Question, Rejected> {
    let message = str_at(params, "message").unwrap_or_default().to_owned();
    let form = match str_at(params, "mode").unwrap_or("form") {
        "url" => match str_at(params, "url") {
            Some(url) => Form::Url(url.to_owned()),
            None => return Err(Box::new((reply, "url question has no url".into()))),
        },
        "form" => match params.get("requestedSchema").map(parse_fields) {
            Some(Ok(fields)) => Form::Fields(fields),
            Some(Err(why)) => return Err(Box::new((reply, why))),
            None => return Err(Box::new((reply, "question has no form".into()))),
        },
        other => {
            return Err(Box::new((
                reply,
                format!("Toasty cannot show a `{other}` question"),
            )));
        }
    };
    let blocking = params
        .pointer("/_meta/codex/delivery")
        .and_then(Value::as_str)
        != Some("async");
    Ok(Question {
        key,
        tool_call_id: str_at(params, "toolCallId").map(str::to_owned),
        message,
        form,
        blocking,
        reply,
    })
}

fn parse_fields(schema: &Value) -> Result<Vec<Field>, String> {
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let empty = Map::new();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    properties
        .iter()
        .map(|(id, p)| {
            let kind =
                field_kind(p).ok_or_else(|| format!("Toasty cannot show the field `{id}`: {p}"))?;
            Ok(Field {
                id: id.clone(),
                title: str_at(p, "title").unwrap_or(id).to_owned(),
                description: str_at(p, "description").map(str::to_owned),
                required: required.contains(&id.as_str()),
                kind,
            })
        })
        .collect()
}

fn field_kind(p: &Value) -> Option<FieldKind> {
    match str_at(p, "type")? {
        "string" => Some(choices(p).map_or(FieldKind::Text, FieldKind::Choice)),
        "number" => Some(FieldKind::Number { integer: false }),
        "integer" => Some(FieldKind::Number { integer: true }),
        "boolean" => Some(FieldKind::Boolean),
        "array" => choices(p.get("items")?).map(FieldKind::MultiChoice),
        _ => None,
    }
}

/// Reads the choices of a choice field, written either as `oneOf`/`anyOf` entries with
/// `const` and `title`, or as an `enum` with optional `enumNames`.
fn choices(p: &Value) -> Option<Vec<Choice>> {
    if let Some(entries) = p
        .get("oneOf")
        .or_else(|| p.get("anyOf"))
        .and_then(Value::as_array)
    {
        return entries
            .iter()
            .map(|e| {
                let value = str_at(e, "const")?;
                Some(Choice {
                    value: value.to_owned(),
                    label: str_at(e, "title").unwrap_or(value).to_owned(),
                    description: str_at(e, "description").map(str::to_owned),
                })
            })
            .collect();
    }
    let values = p.get("enum")?.as_array()?;
    let names = p.get("enumNames").and_then(Value::as_array);
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let value = v.as_str()?;
            let label = names
                .and_then(|n| n.get(i))
                .and_then(Value::as_str)
                .unwrap_or(value);
            Some(Choice {
                value: value.to_owned(),
                label: label.to_owned(),
                description: None,
            })
        })
        .collect()
}
