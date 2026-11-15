//! A separate process watches the daemon lifetime socket and kills the adapter group.
//! Handlers are installed before any descendants exist, including during startup races.
use rustix::process::{Signal, getpgrp, kill_process_group};
use std::process::Stdio;
use tokio::process::Command;
use tokio::signal::unix::{SignalKind, signal};

pub async fn run() -> anyhow::Result<()> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut args = std::env::args_os().skip(2);
    let fd: i32 = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing lifetime descriptor"))?
        .to_string_lossy()
        .parse()?;
    let mut lifetime = lifetime_socket(fd)?;
    let program = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing adapter executable"))?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;
    let mut byte = [0u8; 1];
    tokio::select! {
        _ = tokio::io::AsyncReadExt::read(&mut lifetime, &mut byte) => {},
        _ = terminate.recv() => {},
        _ = interrupt.recv() => {},
        _ = child.wait() => {},
    }
    // Includes this supervisor. No descendant is left holding the ACP pipes open.
    kill_process_group(getpgrp(), Signal::KILL)?;
    Ok(())
}

#[allow(unsafe_code)]
fn lifetime_socket(fd: i32) -> anyhow::Result<tokio::net::UnixStream> {
    use std::os::fd::FromRawFd;
    anyhow::ensure!(fd > 2, "invalid lifetime descriptor");
    // SAFETY: the internal launcher transfers this descriptor exclusively to us.
    let stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
    rustix::io::fcntl_setfd(&stream, rustix::io::FdFlags::CLOEXEC)?;
    stream.set_nonblocking(true)?;
    Ok(tokio::net::UnixStream::from_std(stream)?)
}
