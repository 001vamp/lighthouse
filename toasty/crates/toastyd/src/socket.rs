//! Owner-only Unix socket with bounded frames and event-driven subscriptions.
use crate::error::{Error, Result};
use crate::service::Service;
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use toasty_proto::{
    ClientMessage, Command, ErrorCode, Failure, Hello, MAX_FRAME_BYTES, PROTOCOL_VERSION, Reply,
    ServerFrame,
};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

pub struct BoundSocket {
    pub listener: UnixListener,
    path: PathBuf,
    _lock: File,
}
impl BoundSocket {
    pub fn bind(directory: &Path) -> Result<Self> {
        private_directory(directory)?;
        let lock = lock_file(&directory.join("daemon.lock"))?;
        let path = directory.join("daemon.sock");
        match std::fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_socket()
                    && metadata.uid() == rustix::process::getuid().as_raw() =>
            {
                std::fs::remove_file(&path)?
            }
            Ok(_) => {
                return Err(Error::Invalid(
                    "refusing to replace a non-socket or foreign-owned daemon.sock".into(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let listener = std::os::unix::net::UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener: UnixListener::from_std(listener)?,
            path,
            _lock: lock,
        })
    }
}
impl Drop for BoundSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn lock_file(path: &Path) -> Result<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)?;
    let metadata = lock.metadata()?;
    if metadata.uid() != rustix::process::getuid().as_raw() || !metadata.is_file() {
        return Err(Error::Invalid(
            "daemon lock must be an owner-controlled regular file".into(),
        ));
    }
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| Error::Conflict(format!("another toastyd holds {}", path.display())))?;
    Ok(lock)
}

pub fn private_directory(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(Error::Invalid(
            "daemon directories must be absolute paths".into(),
        ));
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != rustix::process::getuid().as_raw()
    {
        return Err(Error::Invalid(format!(
            "{} must be a directory owned by your user, without a symlink",
            path.display()
        )));
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub async fn serve(stream: UnixStream, service: Arc<Service>) -> Result<()> {
    if stream.peer_cred()?.uid() != rustix::process::getuid().as_raw() {
        return Err(Error::Invalid(
            "socket caller has a different user ID".into(),
        ));
    }
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    let hello = tokio::time::timeout(Duration::from_secs(10), frame(&mut read))
        .await
        .map_err(|_| Error::Invalid("client did not send a hello within ten seconds".into()))??;
    let hello: Hello =
        serde_json::from_slice(&hello.ok_or_else(|| Error::Invalid("missing hello".into()))?)?;
    if hello.protocol != PROTOCOL_VERSION || hello.client.trim().is_empty() {
        send(
            &mut write,
            &ServerFrame::Error {
                error: Failure {
                    code: ErrorCode::InvalidInput,
                    message: format!(
                        "expected protocol {PROTOCOL_VERSION} and a nonempty client name"
                    ),
                },
            },
        )
        .await?;
        return Ok(());
    }
    send(
        &mut write,
        &ServerFrame::Hello {
            protocol: PROTOCOL_VERSION,
        },
    )
    .await?;
    while let Some(bytes) = frame(&mut read).await? {
        let message: ClientMessage = match serde_json::from_slice(&bytes) {
            Ok(message) => message,
            Err(error) => {
                send(
                    &mut write,
                    &ServerFrame::Error {
                        error: Failure {
                            code: ErrorCode::InvalidInput,
                            message: error.to_string(),
                        },
                    },
                )
                .await?;
                continue;
            }
        };
        if matches!(message.command, Command::Subscribe) {
            let mut events = service.events.subscribe();
            send(
                &mut write,
                &ServerFrame::Response {
                    id: message.id,
                    result: Ok(Reply::Subscribed),
                },
            )
            .await?;
            // This connection becomes a stream. Commands use another connection.
            loop {
                tokio::select! {
                    incoming = read.read_u8() => {
                        match incoming {
                            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                            Err(error) => return Err(error.into()),
                            Ok(_) => {
                                send(&mut write, &ServerFrame::Error { error: Failure { code: ErrorCode::InvalidInput, message: "subscription connections cannot send commands".into() } }).await?;
                                return Ok(());
                            }
                        }
                    }
                    event = events.recv() => match event {
                        Ok(event) => send(&mut write, &ServerFrame::Event { event }).await?,
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            send(&mut write, &ServerFrame::ResyncRequired).await?;
                            return Ok(());
                        }
                        Err(broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                }
            }
        }
        let result = service.execute(message.command).await.map_err(Into::into);
        send(
            &mut write,
            &ServerFrame::Response {
                id: message.id,
                result,
            },
        )
        .await?;
    }
    Ok(())
}
async fn frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let count = reader
        .take((MAX_FRAME_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .await?;
    if count == 0 {
        return Ok(None);
    }
    if count > MAX_FRAME_BYTES || bytes.last() != Some(&b'\n') {
        return Err(Error::Invalid(
            "socket frame exceeds 1 MiB or lacks a newline".into(),
        ));
    }
    Ok(Some(bytes))
}
async fn send<W: AsyncWrite + Unpin, T: Serialize>(write: &mut W, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    tokio::time::timeout(Duration::from_secs(10), write.write_all(&bytes))
        .await
        .map_err(|_| Error::Unavailable("client stopped reading from the socket".into()))??;
    Ok(())
}
