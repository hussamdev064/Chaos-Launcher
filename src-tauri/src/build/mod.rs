//! Docker compilation pipeline (Phase 4).
//!
//! The source checkout and the user's Data.rsdk are bind-mounted read-only; only
//! the chosen output directory is writable. Containers are removed after every
//! build, successful or not. macOS targets use a native host build instead.

pub mod container;
pub mod script;
pub mod target;

pub use target::{BuildTarget, OutputFormat, TargetArchitecture, TargetPlatform};

use std::fmt;
use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildRequest {
    pub target: BuildTarget,
    pub source_path: PathBuf,
    pub data_rsdk_path: PathBuf,
    pub output_directory: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildEvent {
    /// `None` for a log/stage line, `Some(0..=100)` for progress updates.
    pub percent: Option<u8>,
    pub message: String,
}

pub type EventSink = dyn Fn(BuildEvent) + Send + Sync;

#[derive(Debug)]
pub enum BuildError {
    UnsupportedTarget(String),
    UnsupportedHost(String),
    Docker(String),
    Io(String),
    BuildFailed { status: i64, tail: Vec<String> },
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTarget(message) => write!(formatter, "Unsupported target: {message}"),
            Self::UnsupportedHost(message) => write!(formatter, "Unsupported host: {message}"),
            Self::Docker(message) => write!(formatter, "Docker error: {message}"),
            Self::Io(message) => write!(formatter, "Filesystem error: {message}"),
            Self::BuildFailed { status, tail } => {
                writeln!(formatter, "Build process exited with status {status}")?;
                for line in tail {
                    writeln!(formatter, "{line}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for BuildError {}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BuildResult {
    pub output_directory: PathBuf,
    pub artifacts: Vec<PathBuf>,
}

/// Validate, generate the build script, execute it, then list the produced files.
pub async fn run_build(request: BuildRequest, on_event: &EventSink) -> Result<BuildResult, BuildError> {
    request.target.validate()?;
    tokio::fs::create_dir_all(&request.output_directory)
        .await
        .map_err(|error| BuildError::Io(error.to_string()))?;

    on_event(BuildEvent {
        percent: Some(0),
        message: "Preparing build environment.".to_owned(),
    });

    if request.target.platform == TargetPlatform::Macos {
        let native_paths = script::BuildPaths {
            source: request.source_path.clone(),
            data: request.data_rsdk_path.clone(),
            build: std::env::temp_dir().join("chaoslauncher-macos-build"),
            output: request.output_directory.clone(),
        };
        let script = script::build_script(request.target, &native_paths);
        container::run_native(&script, on_event).await?;
    } else {
        let mounts = container::ContainerMounts {
            source_host: request.source_path.clone(),
            data_host: request.data_rsdk_path.clone(),
            output_host: request.output_directory.clone(),
        };
        let script = script::build_script(request.target, &container::container_paths());
        container::run_container(&script, &mounts, on_event).await?;
    }

    on_event(BuildEvent {
        percent: Some(100),
        message: "Collecting artifacts.".to_owned(),
    });

    let mut artifacts = Vec::new();
    let mut entries = tokio::fs::read_dir(&request.output_directory)
        .await
        .map_err(|error| BuildError::Io(error.to_string()))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| BuildError::Io(error.to_string()))?
    {
        artifacts.push(entry.path());
    }
    artifacts.sort();

    Ok(BuildResult {
        output_directory: request.output_directory,
        artifacts,
    })
}
