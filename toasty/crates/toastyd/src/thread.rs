//! Single-writer session lifecycle, durable messages and request settlement.
use crate::error::{Error, Result};
use crate::requests::AgentRequest;
use crate::service::Service;
use std::collections::HashMap;
use std::sync::Arc;
use toasty_backend::{AgentEvent, Session, SessionOptions, SteerOutcome, StopReason};
use toasty_broker::{Answer, Request};
use toasty_proto::{Event, Reply};
use toasty_store::{
    MessageRole, NewThread, RequestId, RequestInput, RequestState, Thread, ThreadId, ThreadStatus,
};
use tokio::sync::{mpsc, oneshot};

pub enum Operation {
    Reply(String),
    Steer(String),
    Cancel,
    Answer(String, Answer),
    Resume,
    Close,
    Shutdown,
}
pub struct Input {
    pub operation: Operation,
    pub reply: oneshot::Sender<Result<Reply>>,
}
struct Pending {
    request: AgentRequest,
    saved: RequestInput,
}
type SteeringReply = (
    oneshot::Sender<Result<Reply>>,
    std::result::Result<SteerOutcome, toasty_backend::SessionError>,
);

pub struct Actor {
    service: Arc<Service>,
    thread: Thread,
    session: Arc<dyn Session>,
    events: mpsc::Receiver<AgentEvent>,
    input: mpsc::Receiver<Input>,
    buffered_text: String,
    pending: HashMap<String, Pending>,
    steering: tokio::task::JoinSet<SteeringReply>,
    turn: Option<
        tokio::task::JoinHandle<std::result::Result<StopReason, toasty_backend::SessionError>>,
    >,
}
impl Actor {
    pub fn spawn(
        service: Arc<Service>,
        id: ThreadId,
        previous: Option<Thread>,
        options: SessionOptions,
        mut input: mpsc::Receiver<Input>,
    ) {
        tokio::spawn(async move {
            let resuming = previous.is_some();
            let cwd = options.cwd.clone();
            let opened = service.open(options).await;
            let setup = async {
                let mut opened = opened?;
                let mut initial = Vec::new();
                let session_id = toasty_store::SessionId::new(opened.session.id().0.clone())?;
                let thread = if let Some(thread) = previous {
                    service.store.set_session_id(&id, session_id).await?;
                    // Resume may fall back to load, which replays cached history. All pre-reply
                    // notifications have been pumped before open returns; do not cache them twice.
                    while let Ok(event) = opened.events.try_recv() {
                        match event {
                            AgentEvent::Permission(_) | AgentEvent::Question(_) => {
                                initial.push(event)
                            }
                            AgentEvent::Exited => {
                                return Err(Error::Unavailable(
                                    "agent exited while reopening history".into(),
                                ));
                            }
                            // Loading replays tools, titles and usage as well as text. The
                            // durable cache is authoritative; only new callbacks remain live.
                            _ => {}
                        }
                    }
                    service
                        .store
                        .set_config(
                            &id,
                            opened
                                .session
                                .config()
                                .into_iter()
                                .map(|c| (c.id, c.current))
                                .collect(),
                        )
                        .await?;
                    service.store.set_status(&id, ThreadStatus::Idle).await?;
                    Thread {
                        status: ThreadStatus::Idle,
                        ..thread
                    }
                } else {
                    let agent = match opened.session.agent() {
                        toasty_backend::AgentKind::Claude => toasty_store::AgentKind::Claude,
                        toasty_backend::AgentKind::Codex => toasty_store::AgentKind::Codex,
                    };
                    service
                        .store
                        .create_thread(NewThread {
                            id: id.clone(),
                            agent,
                            session_id,
                            cwd,
                            config: opened
                                .session
                                .config()
                                .into_iter()
                                .map(|c| (c.id, c.current))
                                .collect(),
                            title: String::new(),
                        })
                        .await?
                };
                Ok::<_, Error>((opened, thread, initial))
            }
            .await;
            match setup {
                Ok((opened, thread, initial)) => {
                    let mut actor = Self {
                        service: service.clone(),
                        thread,
                        session: opened.session,
                        events: opened.events,
                        input,
                        buffered_text: String::new(),
                        pending: HashMap::new(),
                        turn: None,
                        steering: tokio::task::JoinSet::new(),
                    };
                    if let Err(error) = actor.run(initial).await {
                        actor.service.publish(Event::Error {
                            thread_id: actor.thread.id.clone(),
                            error: error.into(),
                        });
                        let _ = actor.status(ThreadStatus::Failed).await;
                    }
                    actor.steering.abort_all();
                    if let Some(turn) = actor.turn.take() {
                        turn.abort();
                    }
                    let _ = actor.withdraw_all().await;
                    actor.session.close().await;
                }
                Err(error) => {
                    let failure: toasty_proto::Failure = error.into();
                    if resuming {
                        let _ = service.store.set_status(&id, ThreadStatus::Failed).await;
                    }
                    service.publish(Event::Error {
                        thread_id: id.clone(),
                        error: failure.clone(),
                    });
                    input.close();
                    while let Some(input) = input.recv().await {
                        let _ = input.reply.send(Err(Error::Reported(failure.clone())));
                    }
                }
            }
            service.remove_closed(&id).await;
        });
    }
    async fn run(&mut self, initial: Vec<AgentEvent>) -> Result<()> {
        for event in initial {
            self.event(event).await?;
        }
        self.publish_thread().await?;
        let mut flush = tokio::time::interval(std::time::Duration::from_millis(25));
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = flush.tick(), if !self.buffered_text.is_empty() => self.flush_text().await?,
                event = self.events.recv() => match event {
                    Some(AgentEvent::Exited) | None => {
                        self.flush_text().await?;
                        return Err(Error::Unavailable("agent process exited".into()));
                    },
                    Some(event) => self.event(event).await?,
                },
                completed = async {
                    match &mut self.turn {
                        Some(turn) => turn.await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.flush_text().await?;
                    self.turn = None;
                    self.withdraw_all().await?;
                    match completed {
                        Ok(Ok(_)) => self.status(ThreadStatus::Idle).await?,
                        Ok(Err(error)) => {
                            self.status(ThreadStatus::Failed).await?;
                            self.service.publish(Event::Error { thread_id: self.thread.id.clone(), error: Error::Session(error).into() });
                        }
                        Err(error) => return Err(Error::Unavailable(format!("turn worker stopped: {error}"))),
                    }
                },
                completed = self.steering.join_next(), if !self.steering.is_empty() => {
                    let Some(completed) = completed else { continue; };
                    self.flush_text().await?;
                    let (reply, outcome) = completed.map_err(|error| Error::Unavailable(format!("steering worker stopped: {error}")))?;
                    let result = match outcome {
                        Ok(SteerOutcome::Injected) => Ok(Reply::Ok),
                        outcome => {
                            self.message(MessageRole::System, "Follow-up was not injected into the running turn.".into()).await?;
                            match outcome {
                                Err(error) => Err(error.into()),
                                _ => Err(Error::Conflict("the turn ended before the follow-up arrived; use reply".into())),
                            }
                        }
                    };
                    let _ = reply.send(result);
                },
                input = self.input.recv() => {
                    self.flush_text().await?;
                    let Some(input) = input else { return Ok(()); };
                    if let Operation::Steer(text) = input.operation {
                        self.steer(text, input.reply).await?;
                        continue;
                    }
                    let stop = matches!(input.operation, Operation::Close | Operation::Shutdown);
                    let result = self.command(input.operation).await;
                    let failed_storage = matches!(result, Err(Error::Store(_)));
                    let _ = input.reply.send(result);
                    if failed_storage { return Err(Error::Unavailable("stopped session after storage failure".into())); }
                    if stop { return Ok(()); }
                }
            }
        }
    }
    async fn command(&mut self, operation: Operation) -> Result<Reply> {
        match operation {
            Operation::Reply(text) => {
                if self.turn.is_some() || !self.steering.is_empty() {
                    return Err(Error::Conflict(
                        "a turn is running; use steer to add a message or cancel it first".into(),
                    ));
                }
                self.message(MessageRole::User, text.clone()).await?;
                self.status(ThreadStatus::Running).await?;
                let session = self.session.clone();
                self.turn = Some(tokio::spawn(async move { session.prompt(text).await }));
                Ok(Reply::Thread(self.thread.clone()))
            }
            Operation::Steer(_) => Err(Error::Invalid(
                "steering must be dispatched by the session worker".into(),
            )),
            Operation::Cancel => {
                if self.turn.is_none() {
                    return Err(Error::Conflict("no turn is running".into()));
                }
                self.session.cancel()?;
                self.withdraw_all().await?;
                Ok(Reply::Ok)
            }
            Operation::Answer(id, answer) => {
                let pending = self
                    .pending
                    .get(&id)
                    .ok_or_else(|| Error::Conflict("request is no longer open".into()))?;
                toasty_broker::validate_answer(&pending.request.payload(), &answer)?;
                let mut saved = pending.saved.clone();
                saved.state = RequestState::Answered(serde_json::to_value(&answer)?);
                let saved = self.service.store.upsert_request(saved).await?;
                if let Some(pending) = self.pending.remove(&id) {
                    pending.request.answer(answer)?;
                }
                self.service.publish(Event::RequestChanged(saved));
                self.refresh_status().await?;
                Ok(Reply::Ok)
            }
            Operation::Resume => Ok(Reply::Thread(self.thread.clone())),
            Operation::Close | Operation::Shutdown => {
                let interrupted = self.turn.is_some();
                self.steering.abort_all();
                self.withdraw_all().await?;
                if let Some(turn) = self.turn.take() {
                    let _ = self.session.cancel();
                    turn.abort();
                }
                self.session.close().await;
                self.status(if matches!(operation, Operation::Close) {
                    ThreadStatus::Closed
                } else if interrupted {
                    ThreadStatus::Interrupted
                } else {
                    self.thread.status
                })
                .await?;
                Ok(Reply::Ok)
            }
        }
    }
    async fn steer(&mut self, text: String, reply: oneshot::Sender<Result<Reply>>) -> Result<()> {
        if self.turn.is_none() || !self.steering.is_empty() {
            let _ = reply.send(Err(Error::Conflict(
                "steering requires a running turn with no other follow-up pending".into(),
            )));
            return Ok(());
        }
        self.message(MessageRole::User, text.clone()).await?;
        let session = self.session.clone();
        self.steering
            .spawn(async move { (reply, session.steer(text).await) });
        Ok(())
    }
    async fn event(&mut self, event: AgentEvent) -> Result<()> {
        if !matches!(&event, AgentEvent::Text { .. }) {
            self.flush_text().await?;
        }
        match event {
            AgentEvent::Flush(done) => {
                let _ = done.send(());
            }
            AgentEvent::Text { role, text, .. } => {
                match role {
                    toasty_backend::Role::User => return Ok(()),
                    toasty_backend::Role::Agent => {}
                    toasty_backend::Role::Thought => return Ok(()),
                };
                self.buffered_text.push_str(&text);
                if self.buffered_text.len() >= 16 * 1024 {
                    self.flush_text().await?;
                }
            }
            AgentEvent::Title(title) => {
                self.service.store.set_title(&self.thread.id, title).await?;
                self.publish_thread().await?;
            }
            AgentEvent::Permission(p) => self.request(AgentRequest::Permission(p)).await?,
            AgentEvent::Question(q) => self.request(AgentRequest::Question(q)).await?,
            AgentEvent::Withdrawn { request } => {
                let ids: Vec<_> = self
                    .pending
                    .iter()
                    .filter(|(_, p)| p.request.key() == request.0)
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in ids {
                    self.withdraw(&id).await?;
                }
                self.refresh_status().await?;
            }
            AgentEvent::ToolCall {
                id,
                title,
                kind,
                status,
            } => self.service.publish(Event::Tool {
                thread_id: self.thread.id.clone(),
                id,
                title: Some(title),
                kind,
                status: status.map(|s| format!("{s:?}")),
            }),
            AgentEvent::ToolCallUpdate { id, title, status } => self.service.publish(Event::Tool {
                thread_id: self.thread.id.clone(),
                id,
                title,
                kind: None,
                status: status.map(|s| format!("{s:?}")),
            }),
            AgentEvent::Usage { used, size } => self.service.publish(Event::Usage {
                thread_id: self.thread.id.clone(),
                used,
                size,
            }),
            AgentEvent::Config(config) => {
                self.service
                    .store
                    .set_config(
                        &self.thread.id,
                        config.into_iter().map(|c| (c.id, c.current)).collect(),
                    )
                    .await?;
                self.publish_thread().await?;
            }
            AgentEvent::Exited => {}
        }
        Ok(())
    }
    async fn request(&mut self, request: AgentRequest) -> Result<()> {
        let id = self.service.id();
        let model = Request {
            id: id.clone(),
            thread_id: self.thread.id.as_str().into(),
            payload: request.payload(),
        };
        toasty_broker::validate_request(&model)?;
        let saved = RequestInput {
            id: RequestId::new(id.clone())?,
            thread_id: self.thread.id.clone(),
            payload: serde_json::to_value(&model.payload)?,
            state: RequestState::Open,
        };
        let stored = self.service.store.upsert_request(saved.clone()).await?;
        self.pending.insert(id, Pending { request, saved });
        self.service.publish(Event::RequestChanged(stored));
        self.refresh_status().await
    }
    async fn withdraw(&mut self, id: &str) -> Result<()> {
        if let Some(pending) = self.pending.get(id) {
            let mut saved = pending.saved.clone();
            saved.state = RequestState::Withdrawn;
            let stored = self.service.store.upsert_request(saved).await?;
            self.pending.remove(id);
            self.service.publish(Event::RequestChanged(stored));
        }
        Ok(())
    }
    async fn withdraw_all(&mut self) -> Result<()> {
        let ids: Vec<_> = self.pending.keys().cloned().collect();
        for id in ids {
            self.withdraw(&id).await?;
        }
        Ok(())
    }
    async fn refresh_status(&mut self) -> Result<()> {
        self.status(if self.pending.values().any(|p| p.request.blocking()) {
            ThreadStatus::Waiting
        } else if self.turn.is_some() {
            ThreadStatus::Running
        } else {
            ThreadStatus::Idle
        })
        .await
    }
    async fn status(&mut self, status: ThreadStatus) -> Result<()> {
        self.service
            .store
            .set_status(&self.thread.id, status)
            .await?;
        self.publish_thread().await
    }
    async fn publish_thread(&mut self) -> Result<()> {
        self.thread = self
            .service
            .store
            .get_thread(&self.thread.id)
            .await?
            .ok_or_else(|| Error::NotFound("thread vanished from storage".into()))?;
        self.service
            .publish(Event::ThreadChanged(self.thread.clone()));
        Ok(())
    }
    async fn flush_text(&mut self) -> Result<()> {
        if !self.buffered_text.is_empty() {
            let text = std::mem::take(&mut self.buffered_text);
            self.message(MessageRole::Agent, text).await?;
        }
        Ok(())
    }
    async fn message(&self, role: MessageRole, text: String) -> Result<()> {
        let message = self
            .service
            .store
            .append_message(&self.thread.id, role, text)
            .await?;
        self.service.publish(Event::Message(message));
        Ok(())
    }
}
