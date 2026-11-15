//! Starting and stopping adapter processes so none outlives the daemon.
//!
//! Being a child process is not enough: in the spike, Codex's adapter kept running after its
//! parent was killed. So each adapter:
//!
//! - leads its own process group, so the adapter and everything it starts can be stopped
//!   together;
//! - has a lifetime socket; EOF tells the supervisor to kill the group;
//! - is written down in a registry folder, so a daemon that was killed outright can stop the
//!   group it left behind the next time it starts.

use std::ffi::OsString;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use rustix::process::{Pid, Signal};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// How long an adapter gets to exit after its input closes, before it is killed.
const GRACE: Duration = Duration::from_secs(3);

/// Why an adapter could not be started.
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("could not start {program}: {source}")]
    Start {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{program} started without the pipes Toasty needs")]
    NoPipes { program: String },
    #[error("could not write the process registry at {path}: {source}")]
    Registry {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// What to run: the program, its arguments and extra environment.
#[derive(Debug, Clone)]
pub struct AdapterCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub cwd: PathBuf,
    /// Short name for logs, such as `claude`.
    pub label: String,
}

/// A folder holding one file per running adapter group. Each file is named after the group
/// id and holds the leader's start time, so a reused process id is never mistaken for ours.
#[derive(Debug, Clone)]
pub struct Registry {
    dir: PathBuf,
    supervisor: Option<PathBuf>,
}

impl Registry {
    /// Uses `dir`, creating it owner-only if needed.
    pub fn open(dir: PathBuf) -> Result<Self, SpawnError> {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|source| SpawnError::Registry {
                path: dir.clone(),
                source,
            })?;
        Ok(Self {
            dir,
            supervisor: None,
        })
    }

    /// Run adapters through this executable's internal process-group supervisor.
    pub fn with_supervisor(mut self, program: PathBuf) -> Self {
        self.supervisor = Some(program);
        self
    }

    /// Kills every adapter group left behind by a daemon that did not shut down cleanly.
    /// Returns how many groups were stopped.
    pub fn reap_leftovers(&self) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let mut reaped = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let recorded = std::fs::read_to_string(&path).ok();
            let pid = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<i32>().ok())
                .and_then(Pid::from_raw);
            if let (Some(pid), Some(recorded)) = (pid, recorded)
                && start_time(pid).as_deref() == Some(recorded.trim())
            {
                tracing::info!(
                    pgid = pid.as_raw_nonzero().get(),
                    "stopping a leftover adapter group"
                );
                let _ = rustix::process::kill_process_group(pid, Signal::KILL);
                reaped += 1;
            }
            let _ = std::fs::remove_file(&path);
        }
        reaped
    }

    fn record(&self, pid: Pid) -> Result<PathBuf, SpawnError> {
        let path = self.dir.join(pid.as_raw_nonzero().get().to_string());
        let stamp = start_time(pid).unwrap_or_default();
        std::fs::write(&path, stamp).map_err(|source| SpawnError::Registry {
            path: path.clone(),
            source,
        })?;
        Ok(path)
    }
}

/// The start time of a process, in clock ticks since boot, from `/proc/<pid>/stat`.
fn start_time(pid: Pid) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid.as_raw_nonzero())).ok()?;
    // The command name is in parentheses and may hold spaces, so count from the last ')'.
    // Field 22 overall is the start time; after the ')' that is the 20th field.
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(19).map(str::to_owned)
}

/// A running adapter. Dropping it kills the whole group at once; [`AdapterProcess::shutdown`]
/// first gives it time to exit on its own.
pub struct AdapterProcess {
    child: Child,
    _lifetime: Option<UnixStream>,
    pgid: Option<Pid>,
    record: Option<PathBuf>,
    label: String,
}

impl AdapterProcess {
    /// Starts the adapter in its own process group. Returns its input and output pipes.
    /// Its error output goes to the daemon's log.
    pub fn spawn(
        cmd: &AdapterCommand,
        registry: &Registry,
    ) -> Result<(Self, ChildStdin, ChildStdout), SpawnError> {
        let program = cmd.program.display().to_string();
        let lifetime = if registry.supervisor.is_some() {
            Some(UnixStream::pair().map_err(|source| SpawnError::Start {
                program: program.clone(),
                source,
            })?)
        } else {
            None
        };
        let mut command = if let Some(supervisor) = &registry.supervisor {
            let mut command = Command::new(supervisor);
            command.arg("--supervise-adapter");
            if let Some((_, reader)) = &lifetime {
                command.arg(reader.as_raw_fd().to_string());
            }
            command.arg(&cmd.program);
            command
        } else {
            Command::new(&cmd.program)
        };
        command
            .args(&cmd.args)
            .envs(cmd.env.iter().map(|(k, v)| (k, v)))
            .current_dir(&cmd.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        if let Some((_, reader)) = &lifetime {
            inherit_lifetime(&mut command, reader);
        } else {
            set_parent_death_signal(&mut command);
        }
        let mut child = command.spawn().map_err(|source| SpawnError::Start {
            program: program.clone(),
            source,
        })?;
        let lifetime = lifetime.map(|(writer, _reader)| writer);
        let pgid = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .and_then(Pid::from_raw);
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(SpawnError::NoPipes { program });
        };
        let record = match pgid {
            Some(pid) => match registry.record(pid) {
                Ok(record) => Some(record),
                Err(error) => {
                    let _ = rustix::process::kill_process_group(pid, Signal::KILL);
                    return Err(error);
                }
            },
            None => None,
        };
        let label = cmd.label.clone();
        tokio::spawn(log_stderr(label.clone(), stderr));
        Ok((
            Self {
                child,
                _lifetime: lifetime,
                pgid,
                record,
                label,
            },
            stdin,
            stdout,
        ))
    }

    /// Waits up to a few seconds for the adapter to exit (its input should already be
    /// closed), then kills whatever is left of its group.
    pub async fn shutdown(mut self) {
        if let Some(pid) = self.pgid {
            let _ = rustix::process::kill_process_group(pid, Signal::TERM);
        }
        if tokio::time::timeout(GRACE, self.child.wait())
            .await
            .is_err()
        {
            tracing::warn!(adapter = %self.label, "adapter did not exit in time; killing it");
        }
        // Drop kills the group and removes the registry file.
    }

    /// Waits until the adapter exits by itself.
    pub async fn wait(&mut self) {
        let _ = self.child.wait().await;
    }
}

impl Drop for AdapterProcess {
    fn drop(&mut self) {
        if let Some(pid) = self.pgid {
            // Kills the adapter and anything it started, such as `codex app-server`.
            let _ = rustix::process::kill_process_group(pid, Signal::KILL);
        }
        if let Some(path) = &self.record {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Preserve only the supervisor's reader across exec. The writer remains close-on-exec.
#[allow(unsafe_code)]
fn inherit_lifetime(command: &mut Command, reader: &UnixStream) {
    let fd = reader.as_raw_fd();
    // SAFETY: the descriptor remains open through spawn; fcntl is async-signal-safe.
    unsafe {
        command.pre_exec(move || {
            let fd = std::os::fd::BorrowedFd::borrow_raw(fd);
            rustix::io::fcntl_setfd(fd, rustix::io::FdFlags::empty()).map_err(std::io::Error::from)
        });
    }
}

/// Fallback for callers that explicitly run without a supervisor.
#[allow(unsafe_code)]
fn set_parent_death_signal(command: &mut Command) {
    let signal = Signal::KILL;
    let parent = rustix::process::getpid();
    // SAFETY: the closure runs in the child between fork and exec. It only calls prctl,
    // which is async-signal-safe, does not allocate, and touches no shared state.
    unsafe {
        command.pre_exec(move || {
            rustix::process::set_parent_process_death_signal(Some(signal))
                .map_err(std::io::Error::from)?;
            if rustix::process::getppid() != Some(parent) {
                return Err(std::io::Error::from_raw_os_error(3));
            }
            Ok(())
        });
    }
}

async fn log_stderr(label: String, stderr: tokio::process::ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(adapter = %label, "{line}");
    }
}

/// Finds `name` on `PATH`, the way a shell would.
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
