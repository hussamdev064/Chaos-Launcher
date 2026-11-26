//! Target selection shared with the frontend and validated in the backend.
//! Serde names match the values emitted by `src/ui/targetSelector.ts`.

use serde::{Deserialize, Serialize};

use super::BuildError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetPlatform {
    Windows,
    Linux,
    Android,
    Macos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetArchitecture {
    #[serde(rename = "x86_64")]
    X86_64,
    #[serde(rename = "aarch64")]
    Aarch64,
    #[serde(rename = "armv7")]
    Armv7,
    #[serde(rename = "universal")]
    Universal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Exe,
    Portable,
    Elf,
    AppImage,
    Apk,
    App,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildTarget {
    pub platform: TargetPlatform,
    pub architecture: TargetArchitecture,
    pub format: OutputFormat,
}

impl BuildTarget {
    /// Reject combinations the builder image cannot produce instead of failing
    /// deep inside CMake with an opaque compiler error.
    pub fn validate(&self) -> Result<(), BuildError> {
        use TargetArchitecture as Arch;
        use TargetPlatform as Platform;

        let (valid_arch, valid_format) = match self.platform {
            Platform::Windows => (self.architecture == Arch::X86_64, {
                matches!(self.format, OutputFormat::Exe | OutputFormat::Portable)
            }),
            Platform::Linux => (self.architecture == Arch::X86_64, {
                matches!(self.format, OutputFormat::Elf | OutputFormat::AppImage)
            }),
            Platform::Android => (
                matches!(self.architecture, Arch::Aarch64 | Arch::Armv7),
                self.format == OutputFormat::Apk,
            ),
            Platform::Macos => (
                matches!(self.architecture, Arch::Universal | Arch::X86_64 | Arch::Aarch64),
                self.format == OutputFormat::App,
            ),
        };

        if !valid_arch || !valid_format {
            return Err(BuildError::UnsupportedTarget(format!(
                "{:?}/{:?}/{:?} is not available with the current builder image",
                self.platform, self.architecture, self.format
            )));
        }

        if self.platform == Platform::Macos && !cfg!(target_os = "macos") {
            return Err(BuildError::UnsupportedHost(
                "macOS .app bundles require a macOS host: Apple SDK rules prevent Linux container cross-compilation".to_owned(),
            ));
        }

        Ok(())
    }

    /// CMake arguments shared by every CMake-based target. Plus content stays
    /// disabled unless the user explicitly opts in; revision 5 is always pinned.
    /// Upstream names the engine binary `RSDKv5`, so we override it to a stable
    /// `SonicMania` name.
    pub fn common_cmake_args(include_plus: bool) -> Vec<&'static str> {
        // Kept as an explicit CMake flag so the safe default remains auditable.
        let disable_plus_arg = if include_plus {
            "-DRETRO_DISABLE_PLUS=OFF"
        } else {
            "-DRETRO_DISABLE_PLUS=ON"
        };
        vec![
            "-DCMAKE_BUILD_TYPE=Release",
            "-DRETRO_REVISION=5",
            disable_plus_arg,
            "-DWITH_RSDK=ON",
            "-DGAME_STATIC=ON",
            "-DRETRO_OUTPUT_NAME=SonicMania",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_windows_exe() {
        let target = BuildTarget {
            platform: TargetPlatform::Windows,
            architecture: TargetArchitecture::X86_64,
            format: OutputFormat::Exe,
        };
        assert!(target.validate().is_ok());
    }

    #[test]
    fn rejects_android_app_bundle() {
        let target = BuildTarget {
            platform: TargetPlatform::Android,
            architecture: TargetArchitecture::Aarch64,
            format: OutputFormat::App,
        };
        assert!(matches!(target.validate(), Err(BuildError::UnsupportedTarget(_))));
    }
}
