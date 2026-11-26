//! Small, local-only settings and output bookkeeping.
//!
//! Preferences live in the launcher's own cache directory. The output manifest
//! records only directories ChaosLauncher created, which lets Full Clean remove
//! generated builds without ever deleting an entire user-chosen folder.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const SETTINGS_FILE: &str = "launcher-settings.json";
const MANIFEST_FILE: &str = "output-manifest.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherSettings {
    pub output_path: Option<String>,
    /// Opt-in only: older settings files are treated as disabled, which keeps
    /// `-DRETRO_DISABLE_PLUS=ON` as the launcher's default for legal compliance.
    #[serde(default)]
    pub include_plus: bool,
    /// Local-only convenience: copy the selected Data.rsdk into a generated
    /// build output on this machine. Never embedded in launcher releases.
    #[serde(default)]
    pub include_data_rsdk: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputManifest {
    pub entries: Vec<OutputEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputEntry {
    /// One timestamped build directory, never the custom output root itself.
    pub path: PathBuf,
    /// The managed or custom root that `path` was created under.
    pub root: PathBuf,
}

pub fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_cache_dir()
        .map(|path| path.join(SETTINGS_FILE))
        .map_err(|error| error.to_string())
}

pub fn manifest_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_cache_dir()
        .map(|path| path.join(MANIFEST_FILE))
        .map_err(|error| error.to_string())
}

pub fn load_settings(app: &AppHandle) -> Result<LauncherSettings, String> {
    read_json(&settings_path(app)?)?.ok_or_else(|| "Settings file is unreadable.".to_owned())
        .or_else(|_| Ok(LauncherSettings::default()))
}

pub fn save_settings(app: &AppHandle, settings: &LauncherSettings) -> Result<(), String> {
    write_json(&settings_path(app)?, settings)
}

pub fn remember_output(app: &AppHandle, path: &Path, root: &Path) -> Result<(), String> {
    let path = absolute_path(path)?;
    let root = absolute_path(root)?;
    if !is_safe_output_child(&path, &root) {
        return Err(format!(
            "Refusing to track an output outside its root: {}",
            path.display()
        ));
    }

    let mut manifest = load_manifest(app)?;
    let entry = OutputEntry { path, root };
    if !manifest.entries.contains(&entry) {
        manifest.entries.push(entry);
        write_json(&manifest_path(app)?, &manifest)?;
    }
    Ok(())
}

pub fn tracked_outputs(app: &AppHandle) -> Result<Vec<OutputEntry>, String> {
    let manifest = load_manifest(app)?;
    Ok(manifest
        .entries
        .into_iter()
        .filter(|entry| is_safe_output_child(&entry.path, &entry.root) && entry.path.is_dir())
        .collect())
}

pub fn clear_manifest(app: &AppHandle) -> Result<(), String> {
    let path = manifest_path(app)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// Folders chosen in a native picker are absolute; typed paths may be relative
/// and may contain `..`. Normalize them before storing or deleting anything.
pub fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(normalize(path))
    } else {
        let current = std::env::current_dir().map_err(|error| error.to_string())?;
        Ok(normalize(&current.join(path)))
    }
}

/// True only for a direct child with a generated-build-style name. This guard
/// prevents a malformed manifest from deleting a user's root folder or any
/// unrelated sibling directory.
pub fn is_safe_output_child(path: &Path, root: &Path) -> bool {
    if path == root || !path.starts_with(root) || path.parent() != Some(root) {
        return false;
    }
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    name.len() <= 96
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && is_generated_output_name(name)
}

/// Generated directories are `<platform>-<architecture>-<format>-<stamp>`.
/// Requiring that shape means a user file such as `Data.rsdk`, a custom folder,
/// or an unrelated sibling can never be mistaken for a build to delete.
fn is_generated_output_name(name: &str) -> bool {
    let Some((_, stamp)) = name.rsplit_once('-') else {
        return false;
    };
    if stamp.is_empty() || !stamp.chars().all(|character| character.is_ascii_digit()) {
        return false;
    }
    ["windows-", "linux-", "android-", "macos-"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn load_manifest(app: &AppHandle) -> Result<OutputManifest, String> {
    Ok(read_json(&manifest_path(app)?)?
        .unwrap_or_default())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let parsed = serde_json::from_str(&contents).map_err(|error| error.to_string())?;
    Ok(Some(parsed))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temporary = path.with_extension("json.tmp");
    let encoded = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    fs::write(&temporary, encoded).map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_direct_generated_children_are_deletable() {
        let root = Path::new("/home/user/builds");
        assert!(is_safe_output_child(
            &root.join("linux-x86-64-appimage-123"),
            root
        ));
        assert!(!is_safe_output_child(root, root));
        assert!(!is_safe_output_child(&root.join("nested/build"), root));
        assert!(!is_safe_output_child(&root.join("Data.rsdk"), root));
        assert!(!is_safe_output_child(Path::new("/home/user/other/build"), root));
    }

    #[test]
    fn legacy_settings_default_to_plus_disabled() {
        let settings: LauncherSettings =
            serde_json::from_str(r#"{"outputPath":"/home/user/builds"}"#)
                .expect("legacy settings should still deserialize");
        assert_eq!(settings.output_path.as_deref(), Some("/home/user/builds"));
        assert!(!settings.include_plus);
        assert!(!settings.include_data_rsdk);
    }

    #[test]
    fn local_data_copy_opt_in_round_trips() {
        let settings: LauncherSettings = serde_json::from_str(
            r#"{"outputPath":"/home/user/builds","includePlus":true,"includeDataRsdk":true}"#,
        )
        .expect("current settings should deserialize");
        let encoded = serde_json::to_string(&settings).expect("settings should serialize");
        let decoded: LauncherSettings =
            serde_json::from_str(&encoded).expect("settings should round-trip");
        assert_eq!(decoded.output_path.as_deref(), Some("/home/user/builds"));
        assert!(decoded.include_plus);
        assert!(decoded.include_data_rsdk);
    }

    #[test]
    fn relative_paths_are_normalized() {
        let path = absolute_path(Path::new("builds/../other")).expect("path should normalize");
        assert!(path.is_absolute());
        assert!(path.ends_with("other"));
        assert!(!path.to_string_lossy().contains(".."));
    }
}
