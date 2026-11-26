//! Tauri command bridge (Phase 5).
//!
//! The frontend never reads `Data.rsdk` itself: the native picker resolves a
//! real path, Rust validates it, and Rust owns the output directory. Build
//! progress is pushed to `build://event` so the existing progress view can be
//! driven without polling.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_shell::ShellExt;

use crate::build::{
    self, BuildRequest, BuildTarget, OutputFormat, TargetArchitecture, TargetPlatform,
};
use crate::source::{asset, git};

pub const BUILD_EVENT: &str = "build://event";
pub const DOCKER_STATUS_EVENT: &str = "docker://status";

/// Latest startup image status. Cached so a frontend listener attached after
/// startup can still learn what happened via `docker_status`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerStatus {
    pub stage: &'static str,
    pub message: String,
}

impl Default for DockerStatus {
    fn default() -> Self {
        Self {
            stage: "checking",
            message: "Checking Docker...".to_owned(),
        }
    }
}

#[derive(Debug, Default)]
pub struct DockerState {
    status: Mutex<DockerStatus>,
}

/// One launcher operation at a time. Builds mutate the shared cache and Docker
/// daemon, so overlapping runs would produce confusing logs and artifacts.
pub struct BuildState {
    operation: tokio::sync::Mutex<()>,
}

impl Default for BuildState {
    fn default() -> Self {
        Self {
            operation: tokio::sync::Mutex::new(()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatedAsset {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub file_count: u16,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetDto {
    pub platform: TargetPlatform,
    pub architecture: TargetArchitecture,
    pub format: OutputFormat,
}

impl From<TargetDto> for BuildTarget {
    fn from(value: TargetDto) -> Self {
        Self {
            platform: value.platform,
            architecture: value.architecture,
            format: value.format,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartBuildRequest {
    pub asset_path: String,
    pub target: TargetDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildResponse {
    pub output_directory: String,
    pub artifacts: Vec<String>,
    pub asset_sha256: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BuildEventDto {
    percent: Option<u8>,
    message: String,
}

/// Update cached status and emit it. Used by the startup task and by commands.
pub fn publish_docker_status(app: &tauri::AppHandle, stage: &'static str, message: impl Into<String>) {
    let status = DockerStatus {
        stage,
        message: message.into(),
    };
    if let Some(state) = app.try_state::<DockerState>() {
        if let Ok(mut current) = state.status.lock() {
            *current = status.clone();
        }
    }
    let _ = app.emit(DOCKER_STATUS_EVENT, status);
}

#[tauri::command]
pub fn docker_status(state: State<'_, DockerState>) -> Result<DockerStatus, String> {
    state
        .status
        .lock()
        .map(|status| status.clone())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn host_platform() -> &'static str {
    std::env::consts::OS
}

/// Open the native picker and validate the chosen file before returning it to
/// the UI. Nothing is copied: Docker reads the path directly from this machine.
#[tauri::command]
pub async fn choose_data_rsdk(app: tauri::AppHandle) -> Result<Option<ValidatedAsset>, String> {
    let picked = app
        .dialog()
        .file()
        .add_filter("RSDKv5 data", &["rsdk"])
        .set_title("Select Data.rsdk")
        .blocking_pick_file();

    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|error| error.to_string())?;
    Ok(Some(validated_asset(&path).await?))
}

#[tauri::command]
pub async fn validate_asset(path: String) -> Result<ValidatedAsset, String> {
    validated_asset(Path::new(&path)).await
}

/// Exposed for diagnostics and retry flows. `start_build` uses the same helper
/// so a fresh command call after a failure simply reuses the cached checkout.
#[tauri::command]
pub async fn prepare_source(app: tauri::AppHandle) -> Result<String, String> {
    let state = build_state(&app)?;
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Another launcher operation is already running.".to_owned())?;

    let path = prepared_source(&app).await?;
    Ok(path_to_string(&path))
}

#[tauri::command]
pub async fn start_build(
    request: StartBuildRequest,
    app: tauri::AppHandle,
) -> Result<BuildResponse, String> {
    let state = build_state(&app)?;
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Another launcher operation is already running.".to_owned())?;

    let target = BuildTarget::from(request.target);
    // Validate before cloning a large repository if the selection is impossible.
    target.validate().map_err(|error| error.to_string())?;

    let data_path = PathBuf::from(&request.asset_path);
    let validated = validated_asset(&data_path).await?;
    let source_path = prepared_source(&app).await?;
    let output_directory = output_directory(&app, target)?;

    let on_event = build_event_emitter(app.clone());
    on_event(build::BuildEvent {
        percent: Some(0),
        message: "Preparing build environment.".to_owned(),
    });

    let result = build::run_build(
        BuildRequest {
            target,
            source_path,
            data_rsdk_path: data_path,
            output_directory,
        },
        &on_event,
    )
    .await
    .map_err(|error| error.to_string())?;

    Ok(BuildResponse {
        output_directory: path_to_string(&result.output_directory),
        artifacts: result
            .artifacts
            .iter()
            .map(|path| path_to_string(path))
            .collect(),
        asset_sha256: validated.sha256,
    })
}

#[tauri::command]
// The pinned plugin set uses plugin-shell; plugin-opener is deliberately out of scope.
#[allow(deprecated)]
pub async fn open_output_folder(app: tauri::AppHandle, path: String) -> Result<(), String> {
    if !Path::new(&path).is_dir() {
        return Err(format!("Output folder no longer exists: {path}"));
    }
    app.shell()
        .open(path, None)
        .map_err(|error| error.to_string())
}

fn build_state(app: &tauri::AppHandle) -> Result<Arc<BuildState>, String> {
    app.try_state::<Arc<BuildState>>()
        .map(|state| state.inner().clone())
        .ok_or_else(|| "Build state is unavailable.".to_owned())
}

async fn validated_asset(path: &Path) -> Result<ValidatedAsset, String> {
    // No expected hash is bundled; SHA-256 is computed for logging/identity only.
    let info = asset::validate_data_rsdk(path, None)
        .await
        .map_err(|error| error.to_string())?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "The selected path has no usable file name.".to_owned())?;

    Ok(ValidatedAsset {
        name: name.to_owned(),
        path: path_to_string(path),
        size_bytes: info.size_bytes,
        sha256: info.sha256,
        file_count: info.file_count,
    })
}

async fn prepared_source(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?;
    let on_progress = source_event_emitter(app.clone());
    on_progress("Preparing source checkout...");
    git::prepare_source(&cache, on_progress)
        .await
        .map_err(|error| error.to_string())
}

/// Each build gets a fresh directory so a failed run cannot contaminate the
/// artifacts of a later one. Names contain only fixed slugs and a timestamp.
fn output_directory(app: &tauri::AppHandle, target: BuildTarget) -> Result<PathBuf, String> {
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();

    Ok(cache.join("outputs").join(format!(
        "{}-{}-{}-{}",
        platform_slug(target.platform),
        architecture_slug(target.architecture),
        format_slug(target.format),
        stamp
    )))
}

fn build_event_emitter(app: tauri::AppHandle) -> impl Fn(build::BuildEvent) + Send + Sync + 'static {
    move |event| {
        let _ = app.emit(
            BUILD_EVENT,
            BuildEventDto {
                percent: event.percent,
                message: event.message,
            },
        );
    }
}

fn source_event_emitter(app: tauri::AppHandle) -> impl Fn(&str) + Send + 'static {
    move |message| {
        let _ = app.emit(
            BUILD_EVENT,
            BuildEventDto {
                percent: None,
                message: message.to_owned(),
            },
        );
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn platform_slug(platform: TargetPlatform) -> &'static str {
    match platform {
        TargetPlatform::Windows => "windows",
        TargetPlatform::Linux => "linux",
        TargetPlatform::Android => "android",
        TargetPlatform::Macos => "macos",
    }
}

fn architecture_slug(architecture: TargetArchitecture) -> &'static str {
    match architecture {
        TargetArchitecture::X86_64 => "x86-64",
        TargetArchitecture::Aarch64 => "arm64",
        TargetArchitecture::Armv7 => "armv7",
        TargetArchitecture::Universal => "universal",
    }
}

fn format_slug(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Exe => "exe",
        OutputFormat::Portable => "portable",
        OutputFormat::Elf => "elf",
        OutputFormat::AppImage => "appimage",
        OutputFormat::Apk => "apk",
        OutputFormat::App => "app",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_slugs_are_filesystem_safe() {
        assert_eq!(platform_slug(TargetPlatform::Windows), "windows");
        assert_eq!(architecture_slug(TargetArchitecture::X86_64), "x86-64");
        assert_eq!(format_slug(OutputFormat::AppImage), "appimage");
    }
}
