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
use crate::clean::{self, CleanPlan, CleanReport, CleanRequest};
use crate::settings::{self, LauncherSettings};
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
    /// Optional user-chosen destination root. A timestamped child is created here.
    pub output_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildResponse {
    pub output_directory: String,
    pub artifacts: Vec<String>,
    pub asset_sha256: String,
    /// True only when the user opted in to a local-only Data.rsdk copy.
    pub data_rsdk_included: bool,
    pub data_rsdk_output_path: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BuildEventDto {
    percent: Option<u8>,
    phase: Option<String>,
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

#[tauri::command]
pub async fn choose_output_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Select output folder")
        .blocking_pick_folder();
    Ok(picked
        .and_then(|folder| folder.into_path().ok())
        .map(|path| path_to_string(&path)))
}

#[tauri::command]
pub fn load_settings(app: tauri::AppHandle) -> Result<LauncherSettings, String> {
    settings::load_settings(&app)
}

#[tauri::command]
pub fn save_settings(
    app: tauri::AppHandle,
    settings: LauncherSettings,
) -> Result<(), String> {
    let cleaned = LauncherSettings {
        output_path: settings
            .output_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(|path| {
                let normalized = settings::absolute_path(Path::new(path))?;
                if normalized.is_file() {
                    return Err(format!("Output path is a file: {}", normalized.display()));
                }
                Ok(path_to_string(&normalized))
            })
            .transpose()?,
        include_plus: settings.include_plus,
        include_data_rsdk: settings.include_data_rsdk,
    };
    settings::save_settings(&app, &cleaned)
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

    let on_event = build_event_emitter(app.clone());
    on_event(build::BuildEvent::phase(
        "validating",
        "Validating Data.rsdk...",
    ));

    let target = BuildTarget::from(request.target);
    // Validate before cloning a large repository if the selection is impossible.
    target.validate().map_err(|error| error.to_string())?;

    // The saved toggles are authoritative: builds cannot accidentally include
    // Plus/DLC content or a local data copy because of a stale UI value. Older
    // settings files deserialize both flags as false.
    let saved_settings = settings::load_settings(&app)?;
    let include_plus = saved_settings.include_plus;
    let include_data_rsdk = saved_settings.include_data_rsdk;
    on_event(build::BuildEvent::log(if include_plus {
        "Plus/DLC content enabled by saved settings."
    } else {
        "Plus/DLC content disabled by default."
    }));
    on_event(build::BuildEvent::log(if include_data_rsdk {
        "Local Data.rsdk copy enabled: the selected file will be placed in the build output for personal local use only."
    } else {
        "Local Data.rsdk copy disabled by default; build output stays clean."
    }));

    let data_path = PathBuf::from(&request.asset_path);
    let validated = validated_asset(&data_path).await?;

    on_event(build::BuildEvent::phase(
        "environment",
        "Preparing source checkout...",
    ));
    let source_path = prepared_source(&app).await?;

    on_event(build::BuildEvent::phase(
        "environment",
        "Preparing build environment...",
    ));
    let (output_directory, output_root) =
        output_directory(&app, target, request.output_path.as_deref())?;
    settings::remember_output(&app, &output_directory, &output_root)?;

    let result = build::run_build(
        BuildRequest {
            target,
            source_path,
            data_rsdk_path: data_path,
            output_directory,
            include_plus,
            include_data_rsdk,
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
        data_rsdk_included: result.data_rsdk_output.is_some(),
        data_rsdk_output_path: result
            .data_rsdk_output
            .as_ref()
            .map(|path| path_to_string(path)),
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

/// Read-only preview shown in the Full Clean confirmation dialog.
#[tauri::command]
pub async fn clean_preview(app: tauri::AppHandle) -> Result<CleanPlan, String> {
    Ok(clean::clean_preview(&app).await)
}

#[tauri::command]
pub async fn full_clean(
    request: CleanRequest,
    app: tauri::AppHandle,
) -> Result<CleanReport, String> {
    let state = build_state(&app)?;
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Another launcher operation is already running.".to_owned())?;

    let on_event = build_event_emitter(app.clone());
    on_event(build::BuildEvent::phase(
        "cleaning",
        "Full Clean started.",
    ));
    Ok(clean::full_clean(&app, request, &on_event).await)
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
/// artifacts of a later one. A custom root receives a timestamped child but is
/// never deleted by Full Clean; only that child is tracked for cleanup.
fn output_directory(
    app: &tauri::AppHandle,
    target: BuildTarget,
    custom_root: Option<&str>,
) -> Result<(PathBuf, PathBuf), String> {
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?;
    let root = match custom_root {
        Some(path) => settings::absolute_path(Path::new(path))?,
        None => cache.join("outputs"),
    };
    if root.is_file() {
        return Err(format!("Output root is a file: {}", root.display()));
    }
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let root = root.canonicalize().unwrap_or(root);

    // Never build into the app cache root or the managed source checkout.
    let source_checkout = cache.join("Sonic-Mania-Decompilation");
    if root == cache || root.starts_with(&source_checkout) {
        return Err(format!(
            "Choose an output folder outside the launcher cache/source: {}",
            root.display()
        ));
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let directory = root.join(format!(
        "{}-{}-{}-{}",
        platform_slug(target.platform),
        architecture_slug(target.architecture),
        format_slug(target.format),
        stamp
    ));
    Ok((directory, root))
}

fn build_event_emitter(app: tauri::AppHandle) -> impl Fn(build::BuildEvent) + Send + Sync + 'static {
    move |event| {
        let _ = app.emit(
            BUILD_EVENT,
            BuildEventDto {
                percent: event.percent,
                phase: event.phase,
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
                phase: None,
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
