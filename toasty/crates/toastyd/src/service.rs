//! One actor owns each session. No shared lock is held across agent or database work.
use crate::error::{Error, Result};
use crate::thread::{Actor, Input, Operation};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use toasty_backend::{Backend, SessionOptions, Start};
use toasty_proto::{Command, Event, Reply};
use toasty_store::{AgentKind, Store, Thread, ThreadId, ThreadStatus};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};

pub struct Service {
    pub store: Store,
    pub events: broadcast::Sender<Event>,
    backend: Arc<dyn Backend>,
    actors: Mutex<HashMap<ThreadId, mpsc::Sender<Input>>>,
    nonce: String,
    next: AtomicU64,
}
impl Service {
    pub async fn new(store: Store, backend: Arc<dyn Backend>) -> Result<Arc<Self>> {
        let nonce = tokio::fs::read_to_string("/proc/sys/kernel/random/uuid")
            .await?
            .trim()
            .to_owned();
        let (events, _) = broadcast::channel(1024);
        Ok(Arc::new(Self {
            store,
            backend,
            events,
            actors: Mutex::new(HashMap::new()),
            nonce,
            next: AtomicU64::new(1),
        }))
    }
    pub fn id(&self) -> String {
        format!(
            "{}-{}",
            self.nonce,
            self.next.fetch_add(1, Ordering::Relaxed)
        )
    }
    pub fn publish(&self, event: Event) {
        let _ = self.events.send(event);
    }
    pub async fn recover(self: &Arc<Self>) -> Result<()> {
        self.store.mark_running_interrupted().await?;
        // Old callbacks belong to dead processes. Never present them as answerable.
        for mut request in self.store.list_open_requests(None).await? {
            request.request.state = toasty_store::RequestState::Withdrawn;
            self.store.upsert_request(request.request).await?;
        }
        for thread in self.store.list_threads().await? {
            if thread.status == ThreadStatus::Interrupted {
                let service = self.clone();
                tokio::spawn(async move {
                    if let Err(error) = service
                        .execute(Command::Resume {
                            thread_id: thread.id.clone(),
                        })
                        .await
                    {
                        service.publish(Event::Error {
                            thread_id: thread.id,
                            error: error.into(),
                        });
                    }
                });
            }
        }
        Ok(())
    }
    pub async fn execute(self: &Arc<Self>, command: Command) -> Result<Reply> {
        match command {
            Command::List => Ok(Reply::Threads(self.store.list_threads().await?)),
            Command::Messages {
                thread_id,
                after,
                limit,
            } => Ok(Reply::Messages(
                self.store.messages_since(&thread_id, after, limit).await?,
            )),
            Command::Requests { thread_id } => Ok(Reply::Requests(
                self.store.list_open_requests(thread_id.as_ref()).await?,
            )),
            Command::Run {
                agent,
                cwd,
                prompt,
                config,
            } => {
                nonempty(&prompt)?;
                if !cwd.is_absolute() {
                    return Err(Error::Invalid("working directory must be absolute".into()));
                }
                let cwd = tokio::fs::canonicalize(cwd).await?;
                if !tokio::fs::metadata(&cwd).await?.is_dir() {
                    return Err(Error::Invalid(
                        "working directory must be a directory".into(),
                    ));
                }
                let id = ThreadId::new(self.id())?;
                let options = SessionOptions {
                    agent: backend_agent(agent),
                    cwd,
                    start: Start::New,
                    config,
                };
                let (tx, rx) = mpsc::channel(64);
                self.actors.lock().await.insert(id.clone(), tx.clone());
                Actor::spawn(self.clone(), id, None, options, rx);
                call(&tx, Operation::Reply(prompt)).await
            }
            Command::Subscribe => Err(Error::Invalid("subscribe is handled by the socket".into())),
            other => {
                let (id, operation) = match other {
                    Command::Reply { thread_id, text } => {
                        nonempty(&text)?;
                        (thread_id, Operation::Reply(text))
                    }
                    Command::Steer { thread_id, text } => {
                        nonempty(&text)?;
                        (thread_id, Operation::Steer(text))
                    }
                    Command::Cancel { thread_id } => (thread_id, Operation::Cancel),
                    Command::Answer {
                        thread_id,
                        request_id,
                        answer,
                    } => (thread_id, Operation::Answer(request_id, answer)),
                    Command::Resume { thread_id } => (thread_id, Operation::Resume),
                    Command::Close { thread_id } => (thread_id, Operation::Close),
                    _ => return Err(Error::Invalid("unsupported command".into())),
                };
                let handle = self.handle(&id).await?;
                call(&handle, operation).await
            }
        }
    }
    async fn handle(self: &Arc<Self>, id: &ThreadId) -> Result<mpsc::Sender<Input>> {
        {
            let actors = self.actors.lock().await;
            if let Some(tx) = actors.get(id).filter(|tx| !tx.is_closed()) {
                return Ok(tx.clone());
            }
        }
        let thread = self
            .store
            .get_thread(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("unknown thread {}", id.as_str())))?;
        if thread.status == ThreadStatus::Closed {
            return Err(Error::Conflict("thread is closed".into()));
        }
        let options = resume_options(&thread);
        let mut actors = self.actors.lock().await;
        if let Some(tx) = actors.get(id).filter(|tx| !tx.is_closed()) {
            return Ok(tx.clone());
        }
        let (tx, rx) = mpsc::channel(64);
        actors.insert(id.clone(), tx.clone());
        Actor::spawn(self.clone(), id.clone(), Some(thread), options, rx);
        Ok(tx)
    }
    pub async fn open(&self, options: SessionOptions) -> Result<toasty_backend::Opened> {
        tokio::time::timeout(
            std::time::Duration::from_secs(120),
            self.backend.open(options),
        )
        .await
        .map_err(|_| {
            Error::Unavailable("adapter setup or session opening exceeded two minutes".into())
        })?
        .map_err(Into::into)
    }
    pub async fn remove_closed(&self, id: &ThreadId) {
        let mut actors = self.actors.lock().await;
        if actors.get(id).is_some_and(|tx| tx.is_closed()) {
            actors.remove(id);
        }
    }
    pub async fn shutdown(&self) {
        let actors: Vec<_> = self.actors.lock().await.drain().map(|(_, tx)| tx).collect();
        let mut tasks = tokio::task::JoinSet::new();
        for tx in actors {
            tasks.spawn(async move {
                let _ = call(&tx, Operation::Shutdown).await;
            });
        }
        while tasks.join_next().await.is_some() {}
    }
}
async fn call(tx: &mpsc::Sender<Input>, operation: Operation) -> Result<Reply> {
    let (reply, rx) = oneshot::channel();
    tx.send(Input { operation, reply }).await.map_err(|_| {
        Error::Unavailable("session worker stopped; retry to reopen its saved session".into())
    })?;
    rx.await
        .map_err(|_| Error::Unavailable("session worker stopped before replying".into()))?
}
fn nonempty(text: &str) -> Result<()> {
    if text.trim().is_empty() {
        return Err(Error::Invalid("message cannot be empty".into()));
    }
    Ok(())
}
fn backend_agent(agent: AgentKind) -> toasty_backend::AgentKind {
    match agent {
        AgentKind::Claude => toasty_backend::AgentKind::Claude,
        AgentKind::Codex => toasty_backend::AgentKind::Codex,
    }
}
fn resume_options(thread: &Thread) -> SessionOptions {
    SessionOptions {
        agent: backend_agent(thread.agent),
        cwd: PathBuf::from(&thread.cwd),
        start: Start::Resume(toasty_backend::SessionId(thread.session_id.as_str().into())),
        config: thread.config.clone(),
    }
}
