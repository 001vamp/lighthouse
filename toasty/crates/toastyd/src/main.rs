//! toastyd owns agent sessions and serves clients on a private Unix socket.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod error;
mod requests;
mod service;
mod socket;
mod supervisor;
mod thread;

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use toasty_backend::{AcpBackend, adapters::Adapters, process::Registry};
use toasty_store::Store;

#[derive(Parser)]
#[command(
    version,
    about = "Own coding-agent sessions and serve local Toasty clients"
)]
struct Args {
    /// Private socket and process-registry directory. Defaults to $XDG_RUNTIME_DIR/toasty.
    #[arg(long)]
    runtime_dir: Option<PathBuf>,
    /// Private database and adapter directory. Defaults to $XDG_DATA_HOME/toasty.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Existing pinned adapter installations, if stored separately.
    #[arg(long)]
    adapters_dir: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--supervise-adapter")) {
        return tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(supervisor::run());
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    let runtime = args
        .runtime_dir
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(|p| PathBuf::from(p).join("toasty")))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "XDG_RUNTIME_DIR is missing; pass --runtime-dir with a private absolute directory"
            )
        })?;
    let data = args
        .data_dir
        .or_else(|| {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
                .map(|p| p.join("toasty"))
        })
        .ok_or_else(|| anyhow::anyhow!("HOME and XDG_DATA_HOME are missing; pass --data-dir"))?;
    socket::private_directory(&data)?;
    let _data_lock = socket::lock_file(&data.join("database.lock"))?;
    let socket = socket::BoundSocket::bind(&runtime)?;
    let registry =
        Registry::open(runtime.join("processes"))?.with_supervisor(std::env::current_exe()?);
    let reaped = registry.reap_leftovers();
    let adapters = Adapters::new(args.adapters_dir.unwrap_or_else(|| data.join("adapters")));
    let service = service::Service::new(
        Store::open(data.join("threads.sqlite3")).await?,
        Arc::new(AcpBackend::new(adapters, registry)),
    )
    .await?;
    service.recover().await?;
    tracing::info!(socket = %runtime.join("daemon.sock").display(), reaped, "toastyd ready");
    let mut termination =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut connections = tokio::task::JoinSet::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(128));
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = termination.recv() => break,
            connection = socket.listener.accept() => {
                let (stream, _) = connection?;
                let Ok(slot) = slots.clone().try_acquire_owned() else { continue; };
                let service = service.clone();
                connections.spawn(async move {
                    let _slot = slot;
                    if let Err(error) = socket::serve(stream, service).await { tracing::debug!(%error, "client disconnected"); }
                });
            }
            Some(result) = connections.join_next(), if !connections.is_empty() => {
                if let Err(error) = result { tracing::warn!(%error, "client task stopped"); }
            }
        }
    }
    connections.abort_all();
    // A hanging adapter cannot prevent process shutdown indefinitely.
    if tokio::time::timeout(std::time::Duration::from_secs(15), service.shutdown())
        .await
        .is_err()
    {
        tracing::warn!("timed out while closing agent sessions");
    }
    Ok(())
}
