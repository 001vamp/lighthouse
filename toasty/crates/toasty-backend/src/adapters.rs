//! The pinned ACP adapters: where they live, how to install them, and how to start them.
//!
//! Each adapter is an npm package pinned to the exact version that passed the spike. It is
//! installed once into `<data dir>/adapters/<name>-<version>/` and started with the `node`
//! found on `PATH`, after checking that Node is new enough.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tokio::process::Command;

use crate::AgentKind;
use crate::process::{AdapterCommand, find_on_path};

/// One pinned adapter package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterSpec {
    pub kind: AgentKind,
    /// The npm package name.
    pub package: &'static str,
    /// The exact version Toasty was checked against.
    pub version: &'static str,
    /// The name of the package's command, from its `bin` field.
    pub bin: &'static str,
    /// The oldest Node major version the package supports.
    pub min_node: u32,
}

/// Claude Code's adapter. Passed the spike on 2026-11-11.
pub const CLAUDE: AdapterSpec = AdapterSpec {
    kind: AgentKind::Claude,
    package: "@agentclientprotocol/claude-agent-acp",
    version: "0.106.0",
    bin: "claude-agent-acp",
    min_node: 22,
};

/// Codex's adapter. Passed the spike on 2026-11-11. It declares no Node minimum; 22 matches
/// Claude's so one check covers both.
pub const CODEX: AdapterSpec = AdapterSpec {
    kind: AgentKind::Codex,
    package: "@agentclientprotocol/codex-acp",
    version: "2.6.0",
    bin: "codex-acp",
    min_node: 22,
};

impl AdapterSpec {
    /// The pinned adapter for an agent.
    pub const fn for_agent(kind: AgentKind) -> Self {
        match kind {
            AgentKind::Claude => CLAUDE,
            AgentKind::Codex => CODEX,
        }
    }

    fn folder_name(&self) -> String {
        let name = self.package.rsplit('/').next().unwrap_or(self.package);
        format!("{name}-{}", self.version)
    }
}

/// What is wrong with the adapter setup, in words the owner can act on.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error(
        "Node.js is not installed. Toasty's agent adapters need Node {need} or newer: install the `nodejs` package"
    )]
    NodeMissing { need: u32 },
    #[error("Node.js {found} is too old. Toasty's agent adapters need Node {need} or newer")]
    NodeTooOld { found: String, need: u32 },
    #[error(
        "npm is not installed, so the {package} adapter cannot be installed: install the `npm` package"
    )]
    NpmMissing { package: &'static str },
    #[error("installing {package}@{version} failed: {detail}")]
    InstallFailed {
        package: &'static str,
        version: &'static str,
        detail: String,
    },
    #[error("the installed {package} is broken: {detail}")]
    Broken {
        package: &'static str,
        detail: String,
    },
    #[error("{agent} is not installed: `{command}` was not found on PATH")]
    AgentMissing {
        agent: &'static str,
        command: &'static str,
    },
    #[error("file error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// The installed adapters, under one root folder.
#[derive(Debug, Clone)]
pub struct Adapters {
    root: PathBuf,
    installs: std::sync::Arc<[tokio::sync::Mutex<()>; 2]>,
}

#[derive(Deserialize)]
struct PackageJson {
    version: String,
    #[serde(default)]
    bin: Bin,
}

#[derive(Deserialize, Default)]
#[serde(untagged)]
enum Bin {
    One(String),
    Many(std::collections::BTreeMap<String, String>),
    #[default]
    None,
}

impl Adapters {
    /// Adapters live in `root`, usually `$XDG_DATA_HOME/toasty/adapters`.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            installs: std::sync::Arc::new([
                tokio::sync::Mutex::new(()),
                tokio::sync::Mutex::new(()),
            ]),
        }
    }

    /// The folder holding one pinned adapter.
    pub fn folder(&self, spec: &AdapterSpec) -> PathBuf {
        self.root.join(spec.folder_name())
    }

    /// Makes sure Node is new enough and the pinned adapter is installed, installing it if
    /// needed. Returns the command that starts it in `cwd`.
    pub async fn prepare(
        &self,
        spec: &AdapterSpec,
        cwd: &Path,
    ) -> Result<AdapterCommand, AdapterError> {
        let node = check_node(spec.min_node).await?;
        let script = match self.locate(spec) {
            Ok(script) => script,
            Err(_) => {
                self.install(spec).await?;
                self.locate(spec)?
            }
        };
        let mut env: Vec<(OsString, OsString)> = vec![
            // Toasty's own hooks see this and stay out of sessions Toasty drives.
            ("TOASTY_OWNED".into(), "1".into()),
        ];
        match spec.kind {
            AgentKind::Claude => {
                find_on_path("claude").ok_or(AdapterError::AgentMissing {
                    agent: "Claude Code",
                    command: "claude",
                })?;
            }
            AgentKind::Codex => {
                // The adapter bundles its own Codex. Use the owner's instead, so their login,
                // config and version are the ones that run.
                let codex = find_on_path("codex").ok_or(AdapterError::AgentMissing {
                    agent: "Codex",
                    command: "codex",
                })?;
                env.push(("CODEX_PATH".into(), codex.into_os_string()));
            }
        }
        Ok(AdapterCommand {
            program: node,
            args: vec![script.into_os_string()],
            env,
            cwd: cwd.to_path_buf(),
            label: spec.kind.as_str().to_owned(),
        })
    }

    /// Finds the installed adapter's entry script, checking it is the pinned version.
    pub fn locate(&self, spec: &AdapterSpec) -> Result<PathBuf, AdapterError> {
        let package_dir = self.folder(spec).join("node_modules").join(spec.package);
        let manifest_path = package_dir.join("package.json");
        let text = std::fs::read_to_string(&manifest_path).map_err(|source| AdapterError::Io {
            path: manifest_path.clone(),
            source,
        })?;
        let manifest: PackageJson =
            serde_json::from_str(&text).map_err(|e| AdapterError::Broken {
                package: spec.package,
                detail: format!("{}: {e}", manifest_path.display()),
            })?;
        if manifest.version != spec.version {
            return Err(AdapterError::Broken {
                package: spec.package,
                detail: format!(
                    "found version {}, Toasty is pinned to {}",
                    manifest.version, spec.version
                ),
            });
        }
        let relative = match manifest.bin {
            Bin::One(path) => Some(path),
            Bin::Many(map) => map.get(spec.bin).cloned(),
            Bin::None => None,
        }
        .ok_or_else(|| AdapterError::Broken {
            package: spec.package,
            detail: format!("package.json has no `{}` command", spec.bin),
        })?;
        let script = package_dir.join(relative);
        if !script.is_file() {
            return Err(AdapterError::Broken {
                package: spec.package,
                detail: format!("{} is missing", script.display()),
            });
        }
        Ok(script)
    }

    /// Installs the pinned adapter with npm. It installs into a temporary folder and renames
    /// it into place, so a failed or interrupted install never leaves a half-installed adapter.
    pub async fn install(&self, spec: &AdapterSpec) -> Result<(), AdapterError> {
        let index = match spec.kind {
            AgentKind::Claude => 0,
            AgentKind::Codex => 1,
        };
        let _install = self.installs[index].lock().await;
        if self.locate(spec).is_ok() {
            return Ok(());
        }
        let npm = find_on_path("npm").ok_or(AdapterError::NpmMissing {
            package: spec.package,
        })?;
        let target = self.folder(spec);
        let staging = self.root.join(format!(
            ".{}.partial-{}",
            spec.folder_name(),
            std::process::id()
        ));
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| AdapterError::Io { path, source }
        };
        tokio::fs::create_dir_all(&staging)
            .await
            .map_err(io(&staging))?;
        tracing::info!(
            package = spec.package,
            version = spec.version,
            "installing agent adapter"
        );
        let output = Command::new(npm)
            .kill_on_drop(true)
            .arg("install")
            .arg("--prefix")
            .arg(&staging)
            .args([
                "--save-exact",
                "--no-fund",
                "--no-audit",
                "--loglevel=error",
            ])
            .arg(format!("{}@{}", spec.package, spec.version))
            .output()
            .await
            .map_err(io(&staging))?;
        if !output.status.success() {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(AdapterError::InstallFailed {
                package: spec.package,
                version: spec.version,
                detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        if tokio::fs::try_exists(&target).await.unwrap_or(false) {
            tokio::fs::remove_dir_all(&target)
                .await
                .map_err(io(&target))?;
        }
        tokio::fs::rename(&staging, &target)
            .await
            .map_err(io(&target))
    }
}

/// Finds `node` on `PATH` and checks its major version.
async fn check_node(need: u32) -> Result<PathBuf, AdapterError> {
    let node = find_on_path("node").ok_or(AdapterError::NodeMissing { need })?;
    let output = Command::new(&node)
        .arg("--version")
        .output()
        .await
        .map_err(|_| AdapterError::NodeMissing { need })?;
    let found = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let major = found
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|m| m.parse::<u32>().ok());
    match major {
        Some(major) if major >= need => Ok(node),
        _ => Err(AdapterError::NodeTooOld { found, need }),
    }
}
