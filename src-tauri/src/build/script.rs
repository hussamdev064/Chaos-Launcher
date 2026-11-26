//! Generate the shell script executed inside the builder container (or natively
//! on macOS). All paths are quoted; user-controlled strings never reach a shell.

use std::path::{Path, PathBuf};

use super::target::{BuildTarget, OutputFormat, TargetPlatform};

/// Container paths are fixed; native macOS builds use absolute host paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPaths {
    pub source: PathBuf,
    pub data: PathBuf,
    pub build: PathBuf,
    pub output: PathBuf,
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

/// A tiny transparent PNG so appimagetool always has an icon to embed.
const APPIMAGE_ICON_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";

pub fn build_script(target: BuildTarget, paths: &BuildPaths) -> String {
    let source = shell_quote(&paths.source);
    let data = shell_quote(&paths.data);
    let build = shell_quote(&paths.build);
    let output = shell_quote(&paths.output);

    let mut script = String::new();
    // POSIX-compatible so the same script can run under macOS /bin/sh.
    script.push_str("set -eu\nexec 2>&1\n");
    script.push_str("echo '::progress::5'\n");
    script.push_str("echo '::phase::configuring'\n");
    script.push_str(&format!("SRC={source}\nDATA={data}\nBUILD={build}\nOUT={output}\n"));
    script.push_str("rm -rf \"$BUILD\"\nmkdir -p \"$BUILD\" \"$OUT\"\n");

    match target.platform {
        TargetPlatform::Windows => push_windows_script(&mut script, target.format),
        TargetPlatform::Linux => push_linux_script(&mut script, target.format),
        TargetPlatform::Android => push_android_script(&mut script, target.architecture),
        TargetPlatform::Macos => push_macos_script(&mut script, target),
    }

    script.push_str("echo '::progress::100'\n");
    script
}

fn common_cmake_prefix(extra_args: &[&str]) -> String {
    let mut args = super::target::BuildTarget::common_cmake_args().to_vec();
    args.extend_from_slice(extra_args);
    format!("cmake -S \"$SRC\" -B \"$BUILD\" -G Ninja {}", args.join(" "))
}

fn push_windows_script(script: &mut String, format: OutputFormat) {
    let portable = if format == OutputFormat::Portable { 1 } else { 0 };
    script.push_str(&format!("FORMAT_PORTABLE={portable}\n"));
    script.push_str("echo '::phase::compiling'\n");
    script.push_str("echo '::stage::configuring Windows build'\n");
    // MSVC resolves these DirectX/XAudio imports implicitly; MinGW needs them
    // in the C++ standard libraries. Include CMake's normal MinGW defaults too,
    // since setting this cache entry replaces the generated platform list.
    script.push_str(&common_cmake_prefix(&[
        "-DCMAKE_TOOLCHAIN_FILE=/opt/cmake/mingw-x86_64.cmake",
        "\"-DCMAKE_CXX_STANDARD_LIBRARIES=-lksuser -lxaudio2_8 -lkernel32 -luser32 -lgdi32 -lwinspool -lshell32 -lole32 -loleaut32 -luuid -lcomdlg32 -ladvapi32\"",
    ]));
    script.push('\n');
    script.push_str("echo '::progress::35'\n");
    script.push_str("cmake --build \"$BUILD\" --parallel\n");
    script.push_str("echo '::progress::80'\n");
    script.push_str(r#"BIN=$(find "$BUILD" -type f -name 'SonicMania.exe' -print -quit)
[ -n "$BIN" ] || { echo 'Could not locate SonicMania.exe after build' >&2; exit 3; }
"#);

    script.push_str("echo '::phase::packaging'\n");
    // Portable output bundles only binaries/DLLs: proprietary Data.rsdk is
    // never distributed, even when the user supplied it for this build.
    script.push_str(r#"if [ "${FORMAT_PORTABLE:-0}" = "1" ]; then
  PORTABLE="$OUT/SonicMania-portable"
  mkdir -p "$PORTABLE"
  cp "$BIN" "$PORTABLE/SonicMania.exe"
  find "$BUILD" -type f -name '*.dll' -exec cp {} "$PORTABLE/" \;
else
  cp "$BIN" "$OUT/SonicMania.exe"
fi
"#);
}

fn push_linux_script(script: &mut String, format: OutputFormat) {
    script.push_str("echo '::phase::compiling'\n");
    script.push_str("echo '::stage::configuring Linux build'\n");
    // Upstream defaults to OpenGL/GLFW; the builder image installs SDL2 instead.
    script.push_str(&common_cmake_prefix(&["-DRETRO_SUBSYSTEM=SDL2", "-DUSE_SDL_AUDIO=ON"]));
    script.push('\n');
    script.push_str("echo '::progress::35'\n");
    script.push_str("cmake --build \"$BUILD\" --parallel\n");
    script.push_str("echo '::progress::80'\n");
    script.push_str(r#"BIN=$(find "$BUILD" -type f -name 'SonicMania' -print -quit)
[ -n "$BIN" ] || { echo 'Could not locate SonicMania binary after build' >&2; exit 3; }
"#);

    script.push_str("echo '::phase::packaging'\n");
    if format == OutputFormat::AppImage {
        script.push_str(&r#"APPDIR="$BUILD/AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
cp "$BIN" "$APPDIR/usr/bin/SonicMania"
# Data.rsdk is intentionally not copied into the AppImage: users supply it locally.
printf '%s' 'ICON_PLACEHOLDER' | base64 -d > "$APPDIR/chaoslauncher.png"
cat > "$APPDIR/chaoslauncher.desktop" <<'DESKTOP'
[Desktop Entry]
Name=Sonic Mania
Exec=SonicMania
Icon=chaoslauncher
Type=Application
Categories=Game;
DESKTOP
cat > "$APPDIR/AppRun" <<'APPRUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/SonicMania" "$@"
APPRUN
chmod +x "$APPDIR/AppRun"
# FUSE is unavailable in many containers, so extract-and-run appimagetool.
appimagetool --appimage-extract-and-run "$APPDIR" "$OUT/SonicMania-x86_64.AppImage"
"#.replace("ICON_PLACEHOLDER", APPIMAGE_ICON_BASE64));
    } else {
        script.push_str("cp \"$BIN\" \"$OUT/SonicMania\"\n");
    }
}

fn push_android_script(script: &mut String, architecture: super::target::TargetArchitecture) {
    let abi = match architecture {
        super::target::TargetArchitecture::Aarch64 => "arm64-v8a",
        super::target::TargetArchitecture::Armv7 => "armeabi-v7a",
        // Validation rejects the others before we get here.
        _ => "arm64-v8a",
    };

    script.push_str("echo '::phase::compiling'\n");
    script.push_str(r#"echo '::stage::building Android APK'
# Upstream keeps the Android/Gradle project in the RSDKv5 submodule.
GRADLE_SRC="$SRC/dependencies/RSDKv5/android"
[ -f "$GRADLE_SRC/build.gradle" ] || { echo 'Android Gradle project not found in submodule' >&2; exit 3; }
# The source mount is read-only. Upstream's Android project expects symlinks to
# a writable game checkout, so copy the source and patch only that copy.
ANDROID_SOURCE="$BUILD/android-source"
rm -rf "$ANDROID_SOURCE"
mkdir -p "$ANDROID_SOURCE"
cp -R "$SRC/." "$ANDROID_SOURCE/"
rm -rf "$ANDROID_SOURCE/.git" "$ANDROID_SOURCE/dependencies/RSDKv5/.git"
# Upstream expects libtheora sources that the pinned checkout omits. The image
# carries the approved, vendored upstream source tree.
THEORA_SRC=/opt/android-deps/libtheora
[ -f "$THEORA_SRC/lib/apiwrapper.c" ] || { echo 'Vendored libtheora source not found in builder image' >&2; exit 3; }
mkdir -p "$ANDROID_SOURCE/dependencies/RSDKv5/dependencies/android/libtheora"
cp -R "$THEORA_SRC/." "$ANDROID_SOURCE/dependencies/RSDKv5/dependencies/android/libtheora/"
OGG_SRC=/opt/android-deps/libogg
[ -f "$OGG_SRC/src/bitwise.c" ] || { echo 'Vendored libogg source not found in builder image' >&2; exit 3; }
cp -R "$OGG_SRC/." "$ANDROID_SOURCE/dependencies/RSDKv5/dependencies/android/libogg/"
# The pinned Android platform file hardcodes a different output name.
sed -i 's/set(RETRO_OUTPUT_NAME "RetroEngine")/set(RETRO_OUTPUT_NAME "SonicMania")/' "$ANDROID_SOURCE/dependencies/RSDKv5/platforms/Android.cmake"
# The Gradle project also needs a writable copy.
GRADLE_WORK="$BUILD/android-project"
rm -rf "$GRADLE_WORK"
mkdir -p "$GRADLE_WORK"
cp -R "$GRADLE_SRC/." "$GRADLE_WORK/"
# Match the image's installed NDK/SDK levels so AGP does not auto-download them.
NDK_REV=$(sed -n 's/^Pkg.Revision = //p' "${ANDROID_NDK_HOME:-/opt/android-ndk}/source.properties" | tr -d '\r')
[ -n "$NDK_REV" ] || { echo 'Could not determine installed NDK revision' >&2; exit 3; }
sed -i "s/ndkVersion \"[^\"]*\"/ndkVersion \"$NDK_REV\"/" "$GRADLE_WORK/app/build.gradle"
sed -i 's/compileSdkVersion 33/compileSdkVersion 34/' "$GRADLE_WORK/app/build.gradle"
sed -i 's/targetSdkVersion 33/targetSdkVersion 34/' "$GRADLE_WORK/app/build.gradle"
sed -i 's#arguments = .*#            arguments = ["-DANDROID_STL=c++_shared", "-DPLATFORM=Android", "-DGAME_STATIC=ON", "-DWITH_RSDK=ON", "-DRETRO_OUTPUT_NAME=SonicMania", "-DRETRO_DISABLE_PLUS=$autobuildFlag", "-DRETRO_REVISION=$retroRevision"]#' "$GRADLE_WORK/app/build.gradle"
# Replace the empty upstream jni CMake project with one that builds the game
# and engine from the writable source copy.
cat > "$GRADLE_WORK/app/jni/CMakeLists.txt" <<EOF
cmake_minimum_required(VERSION 3.7)
project(RSDKv5)
set(android 1)
add_subdirectory("$ANDROID_SOURCE" game)
EOF
# Point the Android plugin at the SDK/NDK installed in the builder image.
printf 'sdk.dir=%s\nndk.dir=%s\n' "${ANDROID_HOME:-/opt/android-sdk}" "${ANDROID_NDK_HOME:-/opt/android-ndk}" > "$GRADLE_WORK/local.properties"
cd "$GRADLE_WORK"
chmod +x ./gradlew
"#);
    // Upstream defaults to revision 3 and leaves Plus enabled unless these
    // project properties are present; keep the launcher's pinned safe defaults.
    script.push_str(&format!(
        "./gradlew assembleRelease --no-daemon -PABIFILTERS='{abi}' -PRETRO_DISABLE_PLUS=ON -PRETRO_REVISION=5\n"
    ));
    script.push_str("echo '::phase::packaging'\n");
    script.push_str(r#"echo '::progress::80'
APK=$(find "$GRADLE_WORK" -type f -name '*.apk' -print -quit)
[ -n "$APK" ] || { echo 'Could not locate a built APK' >&2; exit 3; }
cp "$APK" "$OUT/SonicMania.apk"
"#);
}

fn push_macos_script(script: &mut String, target: BuildTarget) {
    let architectures = match target.architecture {
        super::target::TargetArchitecture::Universal => "x86_64 arm64",
        super::target::TargetArchitecture::X86_64 => "x86_64",
        super::target::TargetArchitecture::Aarch64 => "arm64",
        super::target::TargetArchitecture::Armv7 => "armv7",
    };

    // The pinned submodule has no Darwin CMake platform; upstream ships Xcode
    // projects for macOS, so use xcodebuild on the native macOS host.
    script.push_str("echo '::phase::compiling'\n");
    script.push_str("echo '::stage::building macOS app (native Xcode host)'\n");
    script.push_str(&format!(
        "ARCHS='{architectures}'\nXCODE_BUILD=\"$BUILD/xcode\"\nOTHER_CFLAGS='$(inherited) -DRETRO_REVISION=5 -DRETRO_DISABLE_PLUS=1'\n"
    ));
    script.push_str(r#"xcodebuild -quiet -project "$SRC/SonicMania.xcodeproj" -scheme SonicMania \
  -configuration Release -derivedDataPath "$XCODE_BUILD" ONLY_ACTIVE_ARCH=NO ARCHS="$ARCHS" OTHER_CFLAGS="$OTHER_CFLAGS"
echo '::progress::35'
xcodebuild -quiet -project "$SRC/dependencies/RSDKv5/RSDKv5.xcodeproj" -scheme RSDKv5 \
  -configuration Release -derivedDataPath "$XCODE_BUILD" ONLY_ACTIVE_ARCH=NO ARCHS="$ARCHS" OTHER_CFLAGS="$OTHER_CFLAGS"
echo '::phase::packaging'
echo '::progress::80'
APP_SOURCE=$(find "$XCODE_BUILD" -type d -name 'RSDKv5.app' -print -quit)
[ -n "$APP_SOURCE" ] || { echo 'Could not locate built RSDKv5.app' >&2; exit 3; }
APP="$OUT/SonicMania.app"
rm -rf "$APP"
cp -R "$APP_SOURCE" "$APP"
GAME_DYLIB=$(find "$XCODE_BUILD" -type f -name 'Game.dylib' -print -quit)
if [ -n "$GAME_DYLIB" ]; then
  cp "$GAME_DYLIB" "$APP/Contents/MacOS/" 2>/dev/null || true
fi
# Data.rsdk stays with the user's legally obtained copy; never bundle it.
"#);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::target::TargetArchitecture;

    fn paths() -> BuildPaths {
        BuildPaths {
            source: PathBuf::from("/work/source"),
            data: PathBuf::from("/work/Data.rsdk"),
            build: PathBuf::from("/work/build"),
            output: PathBuf::from("/work/out"),
        }
    }

    #[test]
    fn windows_script_uses_mingw_toolchain() {
        let script = build_script(
            BuildTarget {
                platform: TargetPlatform::Windows,
                architecture: TargetArchitecture::X86_64,
                format: OutputFormat::Exe,
            },
            &paths(),
        );
        assert!(script.contains("-DCMAKE_TOOLCHAIN_FILE=/opt/cmake/mingw-x86_64.cmake"));
        assert!(script.contains("-DCMAKE_CXX_STANDARD_LIBRARIES=-lksuser -lxaudio2_8 -lkernel32"));
        assert!(script.contains("-DRETRO_DISABLE_PLUS=ON"));
    }

    #[test]
    fn linux_appimage_script_invokes_appimagetool() {
        let script = build_script(
            BuildTarget {
                platform: TargetPlatform::Linux,
                architecture: TargetArchitecture::X86_64,
                format: OutputFormat::AppImage,
            },
            &paths(),
        );
        assert!(script.contains("appimagetool --appimage-extract-and-run"));
        assert!(!script.contains("ICON_PLACEHOLDER"));
        assert!(!script.contains("cp \"$DATA\""));
    }

    #[test]
    fn android_script_uses_submodule_gradle_project() {
        let script = build_script(
            BuildTarget {
                platform: TargetPlatform::Android,
                architecture: TargetArchitecture::Aarch64,
                format: OutputFormat::Apk,
            },
            &paths(),
        );
        assert!(script.contains("$SRC/dependencies/RSDKv5/android"));
        assert!(script.contains("$BUILD/android-source"));
        assert!(script.contains("/opt/android-deps/libtheora"));
        assert!(script.contains("/opt/android-deps/libogg"));
        assert!(script.contains("add_subdirectory(\"$ANDROID_SOURCE\" game)"));
        assert!(script.contains("-DGAME_STATIC=ON"));
        assert!(script.contains("-DRETRO_OUTPUT_NAME=SonicMania"));
        assert!(script.contains("-PABIFILTERS='arm64-v8a'"));
        assert!(script.contains("NDK_REV="));
        assert!(script.contains("-PRETRO_DISABLE_PLUS=ON"));
        assert!(script.contains("-PRETRO_REVISION=5"));
    }

    #[test]
    fn macos_script_uses_xcode_projects() {
        let script = build_script(
            BuildTarget {
                platform: TargetPlatform::Macos,
                architecture: TargetArchitecture::Universal,
                format: OutputFormat::App,
            },
            &paths(),
        );
        assert!(script.contains("xcodebuild"));
        assert!(script.contains("ARCHS='x86_64 arm64'"));
        assert!(!script.contains("cp \"$DATA\""));
    }
}
