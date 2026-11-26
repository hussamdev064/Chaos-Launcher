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
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Host-side Android staging mirrors the reference `RSDK/v5` layout.
const ANDROID_RSDK_DIRECTORY: &str = "RSDK";
const ANDROID_DATA_DIRECTORY: &str = "v5";
/// Reference-compatible options from `~/Downloads/Sonic/RSDK/v5/Settings.ini`.
const ANDROID_SETTINGS_INI: &str = "; Retro Engine Config File\n\n\
[Game]\n\
language=0\n\
devMenu=y\n\
dllSupport=y\n\n\
[Video]\n\
shaderSupport=y\n\
screenShader=1\n\
maxPixWidth=424\n";
const ANDROID_MODCONFIG_INI: &str = "[Mods]\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildRequest {
    pub target: BuildTarget,
    pub source_path: PathBuf,
    pub data_rsdk_path: PathBuf,
    pub output_directory: PathBuf,
    /// Persisted user opt-in. Defaults to false, which keeps Plus content off.
    pub include_plus: bool,
    /// Local-only opt-in to copy the selected Data.rsdk into the build output.
    pub include_data_rsdk: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildEvent {
    /// `None` for a log/stage line, `Some(0..=100)` for progress updates.
    pub percent: Option<u8>,
    /// Optional work-line stage such as `validating`, `compiling`, or `cleaning`.
    pub phase: Option<String>,
    pub message: String,
}

impl BuildEvent {
    pub fn progress(percent: u8, message: impl Into<String>) -> Self {
        Self {
            percent: Some(percent),
            phase: None,
            message: message.into(),
        }
    }

    pub fn phase(phase: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            percent: None,
            phase: Some(phase.into()),
            message: message.into(),
        }
    }

    pub fn log(message: impl Into<String>) -> Self {
        Self {
            percent: None,
            phase: None,
            message: message.into(),
        }
    }
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
    /// Present only when the user opted in to a local-only Data.rsdk copy.
    pub data_rsdk_output: Option<PathBuf>,
}

/// Validate, generate the build script, execute it, then list the produced files.
pub async fn run_build(request: BuildRequest, on_event: &EventSink) -> Result<BuildResult, BuildError> {
    request.target.validate()?;
    tokio::fs::create_dir_all(&request.output_directory)
        .await
        .map_err(|error| BuildError::Io(error.to_string()))?;

    on_event(BuildEvent::phase(
        "environment",
        "Preparing build environment.",
    ));

    if request.target.platform == TargetPlatform::Macos {
        let native_paths = script::BuildPaths {
            source: request.source_path.clone(),
            data: request.data_rsdk_path.clone(),
            build: std::env::temp_dir().join("chaoslauncher-macos-build"),
            output: request.output_directory.clone(),
        };
        let script = script::build_script(request.target, &native_paths, request.include_plus);
        container::run_native(&script, on_event).await?;
    } else {
        let mounts = container::ContainerMounts {
            source_host: request.source_path.clone(),
            data_host: request.data_rsdk_path.clone(),
            output_host: request.output_directory.clone(),
        };
        let script =
            script::build_script(request.target, &container::container_paths(), request.include_plus);
        container::run_container(&script, &mounts, on_event).await?;
    }

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

    // Host-side copy only: the builder container never receives a writable
    // mount of the original asset, and launcher release bundles never include it.
    let is_android = request.target.platform == TargetPlatform::Android;
    let data_rsdk_output = if request.include_data_rsdk {
        let phase_message = if is_android {
            "Copying your local Data.rsdk into the Android RSDK/v5 staging folder (personal use only; do not share)..."
        } else {
            "Copying your local Data.rsdk beside the build output (personal use only; do not share)..."
        };
        on_event(BuildEvent::phase("local-data", phase_message));
        let copied =
            copy_data_rsdk_into_output(request.target, &request.data_rsdk_path, &request.output_directory)
                .await?;
        on_event(BuildEvent::log(format!(
            "Data.rsdk copied for personal local use only: {} — do not share, distribute, or upload.",
            copied.display()
        )));
        if is_android {
            on_event(BuildEvent::log(
                "Android staging is ready: copy the RSDK folder to shared storage on the device; the APK never contains game data.",
            ));
        }
        Some(copied)
    } else {
        None
    };

    // Include the local-only copy in the reported artifact list, but never place
    // it inside an AppImage/APK/.app bundle or in a launcher release package.
    if let Some(ref copied) = data_rsdk_output {
        artifacts.push(copied.clone());
        if is_android {
            if let Some(parent) = copied.parent() {
                artifacts.push(parent.to_path_buf());
            }
            let host_alias = request.output_directory.join("Data.rsdk");
            if host_alias.exists() {
                artifacts.push(host_alias);
            }
        }
        artifacts.sort();
        artifacts.dedup();
    }

    on_event(BuildEvent::phase("complete", "Build output ready."));

    Ok(BuildResult {
        output_directory: request.output_directory,
        artifacts,
        data_rsdk_output,
    })
}

/// Copy the user-selected asset into a fresh build output on this host.
///
/// The source file is opened read-only and is never moved or modified. A
/// Windows portable build gets the copy inside its portable folder. Android
/// builds stage a reference-compatible `RSDK/v5` folder beside the APK and keep
/// a hard link at the previous root location for compatibility. Every other
/// target keeps the copy beside the produced binary/app bundle in the output root.
async fn copy_data_rsdk_into_output(
    target: BuildTarget,
    source: &Path,
    output_directory: &Path,
) -> Result<PathBuf, BuildError> {
    if target.platform == TargetPlatform::Android {
        return stage_android_data(source, output_directory).await;
    }

    let destination_dir = if target.platform == TargetPlatform::Windows
        && target.format == OutputFormat::Portable
    {
        let portable = output_directory.join("SonicMania-portable");
        if portable.is_dir() {
            portable
        } else {
            output_directory.to_path_buf()
        }
    } else {
        output_directory.to_path_buf()
    };

    tokio::fs::create_dir_all(&destination_dir)
        .await
        .map_err(|error| BuildError::Io(error.to_string()))?;
    let destination = destination_dir.join("Data.rsdk");
    copy_file_create_new(source, &destination).await?;
    Ok(destination)
}

/// Build the host-side Android `RSDK/v5` staging tree from the pinned reference
/// layout. No game data is bundled into the APK or into ChaosLauncher releases.
async fn stage_android_data(source: &Path, output_directory: &Path) -> Result<PathBuf, BuildError> {
    let data_dir = output_directory
        .join(ANDROID_RSDK_DIRECTORY)
        .join(ANDROID_DATA_DIRECTORY);
    let mods_dir = data_dir.join("mods");
    tokio::fs::create_dir_all(&mods_dir)
        .await
        .map_err(|error| BuildError::Io(error.to_string()))?;
    write_if_missing(&data_dir.join("Settings.ini"), ANDROID_SETTINGS_INI)?;
    write_if_missing(&mods_dir.join("modconfig.ini"), ANDROID_MODCONFIG_INI)?;

    let staged = data_dir.join("Data.rsdk");
    copy_file_create_new(source, &staged).await?;

    // Preserve the previous root-level copy without duplicating game data. If
    // hard links are unavailable, fall back to an independent host-side copy.
    let root_alias = output_directory.join("Data.rsdk");
    if !root_alias.exists() {
        if std::fs::hard_link(&staged, &root_alias).is_err() {
            copy_file_create_new(source, &root_alias).await?;
        }
    }

    Ok(staged)
}

fn write_if_missing(path: &Path, contents: &str) -> Result<(), BuildError> {
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, contents).map_err(|error| BuildError::Io(error.to_string()))
}

async fn copy_file_create_new(source: &Path, destination: &Path) -> Result<(), BuildError> {
    if destination.exists() {
        return Err(BuildError::Io(format!(
            "Refusing to overwrite existing game data: {}",
            destination.display()
        )));
    }

    let mut input = tokio::fs::File::open(source)
        .await
        .map_err(|error| BuildError::Io(error.to_string()))?;
    let mut output = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .await
        .map_err(|error| BuildError::Io(error.to_string()))?;
    tokio::io::copy(&mut input, &mut output)
        .await
        .map_err(|error| BuildError::Io(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_data_copy_preserves_the_original_and_never_overwrites() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let base = std::env::temp_dir().join(format!("chaoslauncher-asset-copy-{stamp}"));
        let source_dir = base.join("source");
        let output = base.join("out");
        let portable = output.join("SonicMania-portable");
        std::fs::create_dir_all(&source_dir).expect("source directory should be created");
        std::fs::create_dir_all(&portable).expect("portable directory should be created");

        let source = source_dir.join("Data.rsdk");
        let expected = b"RSDKv5 local copy test".to_vec();
        std::fs::write(&source, &expected).expect("source asset should be written");

        let target = BuildTarget {
            platform: TargetPlatform::Windows,
            architecture: TargetArchitecture::X86_64,
            format: OutputFormat::Portable,
        };
        let copied = copy_data_rsdk_into_output(target, &source, &output)
            .await
            .expect("local copy should succeed");
        let copied_bytes = std::fs::read(&copied).expect("copied asset should be readable");
        let source_bytes = std::fs::read(&source).expect("source should remain readable");
        let overwrite_error = copy_data_rsdk_into_output(target, &source, &output)
            .await
            .expect_err("an existing output copy must never be overwritten");

        std::fs::remove_dir_all(&base).expect("test directory should be removed");

        assert_eq!(copied, portable.join("Data.rsdk"));
        assert_eq!(copied_bytes, expected);
        assert_eq!(source_bytes, expected);
        assert!(
            overwrite_error
                .to_string()
                .contains("Refusing to overwrite existing game data")
        );
    }

    #[tokio::test]
    async fn android_local_copy_stages_reference_layout_without_touching_the_original() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let base = std::env::temp_dir().join(format!("chaoslauncher-android-staging-{stamp}"));
        let source_dir = base.join("source");
        let output = base.join("out");
        std::fs::create_dir_all(&source_dir).expect("source directory should be created");

        let source = source_dir.join("Data.rsdk");
        let expected = b"RSDKv5 Android staging test".to_vec();
        std::fs::write(&source, &expected).expect("source asset should be written");

        let target = BuildTarget {
            platform: TargetPlatform::Android,
            architecture: TargetArchitecture::Aarch64,
            format: OutputFormat::Apk,
        };
        let staged = copy_data_rsdk_into_output(target, &source, &output)
            .await
            .expect("Android staging should succeed");

        let data_dir = output.join("RSDK/v5");
        let settings = std::fs::read_to_string(data_dir.join("Settings.ini"))
            .expect("Settings.ini should be staged");
        let modconfig = std::fs::read_to_string(data_dir.join("mods/modconfig.ini"))
            .expect("modconfig.ini should be staged");
        let staged_bytes = std::fs::read(&staged).expect("staged asset should be readable");
        let alias = output.join("Data.rsdk");
        let alias_bytes = std::fs::read(&alias).expect("root alias should be readable");
        let source_bytes = std::fs::read(&source).expect("source should remain readable");
        let overwrite_error = copy_data_rsdk_into_output(target, &source, &output)
            .await
            .expect_err("an existing staged copy must never be overwritten");

        std::fs::remove_dir_all(&base).expect("test directory should be removed");

        assert_eq!(staged, data_dir.join("Data.rsdk"));
        assert!(settings.contains("devMenu=y"));
        assert!(settings.contains("shaderSupport=y"));
        assert!(settings.contains("maxPixWidth=424"));
        assert_eq!(modconfig, "[Mods]\n");
        assert_eq!(staged_bytes, expected);
        assert_eq!(alias_bytes, expected);
        assert_eq!(source_bytes, expected);
        assert!(
            overwrite_error
                .to_string()
                .contains("Refusing to overwrite existing game data")
        );
    }
}
