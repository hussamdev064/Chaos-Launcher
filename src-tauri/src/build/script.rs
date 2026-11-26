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

pub fn build_script(target: BuildTarget, paths: &BuildPaths, include_plus: bool) -> String {
    let source = shell_quote(&paths.source);
    let data = shell_quote(&paths.data);
    let build = shell_quote(&paths.build);
    let output = shell_quote(&paths.output);
    // Plus is opt-in. The CMake flag is ON/OFF; the Xcode/Gradle compiler define
    // uses the numeric RSDK_AUTOBUILD macro that actually gates Plus content.
    let disable_plus = if include_plus { "OFF" } else { "ON" };
    let rsdk_autobuild = if include_plus { "0" } else { "1" };

    let mut script = String::new();
    // POSIX-compatible so the same script can run under macOS /bin/sh.
    script.push_str("set -eu\nexec 2>&1\n");
    script.push_str("echo '::progress::5'\n");
    script.push_str("echo '::phase::configuring'\n");
    script.push_str(&format!("SRC={source}\nDATA={data}\nBUILD={build}\nOUT={output}\n"));
    script.push_str("rm -rf \"$BUILD\"\nmkdir -p \"$BUILD\" \"$OUT\"\n");

    match target.platform {
        TargetPlatform::Windows => push_windows_script(&mut script, target.format, include_plus),
        TargetPlatform::Linux => push_linux_script(&mut script, target.format, include_plus),
        TargetPlatform::Android => {
            push_android_script(&mut script, target.architecture, disable_plus, rsdk_autobuild)
        }
        TargetPlatform::Macos => push_macos_script(&mut script, target, disable_plus, rsdk_autobuild),
    }

    script.push_str("echo '::progress::100'\n");
    script
}

fn common_cmake_prefix(extra_args: &[&str], include_plus: bool) -> String {
    let mut args = super::target::BuildTarget::common_cmake_args(include_plus);
    args.extend_from_slice(extra_args);
    format!("cmake -S \"$SRC\" -B \"$BUILD\" -G Ninja {}", args.join(" "))
}

fn push_windows_script(script: &mut String, format: OutputFormat, include_plus: bool) {
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
    ], include_plus));
    script.push('\n');
    script.push_str("echo '::progress::35'\n");
    script.push_str("cmake --build \"$BUILD\" --parallel\n");
    script.push_str("echo '::progress::80'\n");
    script.push_str(r#"BIN=$(find "$BUILD" -type f -name 'SonicMania.exe' -print -quit)
[ -n "$BIN" ] || { echo 'Could not locate SonicMania.exe after build' >&2; exit 3; }
"#);

    script.push_str("echo '::phase::packaging'\n");
    // Portable output bundles only binaries/DLLs; an opted-in Data.rsdk copy is
    // added on the host after the container exits, not by this script.
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

fn push_linux_script(script: &mut String, format: OutputFormat, include_plus: bool) {
    script.push_str("echo '::phase::compiling'\n");
    script.push_str("echo '::stage::configuring Linux build'\n");
    // Upstream defaults to OpenGL/GLFW; the builder image installs SDL2 instead.
    script.push_str(&common_cmake_prefix(
        &["-DRETRO_SUBSYSTEM=SDL2", "-DUSE_SDL_AUDIO=ON"],
        include_plus,
    ));
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
# Data.rsdk is intentionally not copied into the AppImage; an opted-in host-side copy goes beside it.
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

fn push_android_script(
    script: &mut String,
    architecture: super::target::TargetArchitecture,
    disable_plus: &str,
    rsdk_autobuild: &str,
) {
    let abi = match architecture {
        super::target::TargetArchitecture::Aarch64 => "arm64-v8a",
        super::target::TargetArchitecture::Armv7 => "armeabi-v7a",
        // Validation rejects the others before we get here.
        _ => "arm64-v8a",
    };

    script.push_str("echo '::phase::compiling'\n");
    script.push_str(&r#"echo '::stage::building Android APK'
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
sed -i 's#arguments = .*#            arguments = ["-DANDROID_STL=c++_shared", "-DPLATFORM=Android", "-DGAME_STATIC=ON", "-DWITH_RSDK=ON", "-DRETRO_MOD_LOADER=ON", "-DRETRO_MOD_LOADER_VER=2", "-DRETRO_OUTPUT_NAME=SonicMania", "-DRETRO_DISABLE_PLUS=DISABLE_PLUS_PLACEHOLDER", "-DRSDK_AUTOBUILD=RSDK_AUTOBUILD_PLACEHOLDER", "-DRETRO_REVISION=5"]#' "$GRADLE_WORK/app/build.gradle"
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
"#
    .replace("DISABLE_PLUS_PLACEHOLDER", disable_plus)
    .replace("RSDK_AUTOBUILD_PLACEHOLDER", rsdk_autobuild),
    );
    // Reference-compatible Android port: mirror ~/Downloads/Sonic's RSDK/v5
    // layout, keep the mod loader on, and request shared-storage access. Only
    // the writable Gradle copy is patched; the read-only source mount is intact.
    script.push_str(r####"# Patch only the writable Gradle copy, never the read-only source mount.
MANIFEST="$GRADLE_WORK/app/src/main/AndroidManifest.xml"
LAUNCHER_JAVA="$GRADLE_WORK/app/src/main/java/org/rems/rsdkv5/Launcher.java"
RSDK_JAVA="$GRADLE_WORK/app/src/main/java/org/rems/rsdkv5/RSDK.java"
[ -f "$MANIFEST" ] || { echo 'AndroidManifest.xml not found in writable Gradle copy' >&2; exit 3; }
[ -f "$LAUNCHER_JAVA" ] || { echo 'Launcher.java not found in writable Gradle copy' >&2; exit 3; }
[ -f "$RSDK_JAVA" ] || { echo 'RSDK.java not found in writable Gradle copy' >&2; exit 3; }
# The reference runtime used legacy external storage on Android 10. Keep that
# declaration while MANAGE_EXTERNAL_STORAGE covers Android 11+.
sed -i 's/android:hardwareAccelerated="true">/android:hardwareAccelerated="true"\n        android:requestLegacyExternalStorage="true"\n        android:appCategory="game">/' "$MANIFEST"
# The engine library is renamed to SonicMania by Android.cmake above.
sed -i 's/android:value="RetroEngine"/android:value="SonicMania"/' "$MANIFEST"
sed -i 's/System.loadLibrary("RetroEngine")/System.loadLibrary("SonicMania")/' "$RSDK_JAVA"
cat > "$GRADLE_WORK/app/src/main/java/org/rems/rsdkv5/ChaosStorage.java" <<'JAVA'
package org.rems.rsdkv5;

import android.Manifest;
import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;
import android.os.Environment;
import android.provider.DocumentsContract;
import android.provider.Settings;
import android.util.Log;
import androidx.core.app.ActivityCompat;
import androidx.documentfile.provider.DocumentFile;
import java.io.File;
import java.io.FileWriter;
import java.io.IOException;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;

/** Shared RSDK/v5 storage for the ChaosLauncher Android port. */
public final class ChaosStorage {
    public static final String RSDK_ROOT = "RSDK";
    public static final String DATA_DIR = "v5";
    public static final int STORAGE_REQUEST = 9105;
    private static final String AUTHORITY = "com.android.externalstorage.documents";

    // Matches the options present in the reference RSDK/v5/Settings.ini without
    // copying any personal value from it.
    private static final String SETTINGS_INI =
        "; Retro Engine Config File\n\n" +
        "[Game]\n" +
        "language=0\n" +
        "devMenu=y\n" +
        "dllSupport=y\n\n" +
        "[Video]\n" +
        "shaderSupport=y\n" +
        "screenShader=1\n" +
        "maxPixWidth=424\n";

    private static final String MODCONFIG_INI = "[Mods]\n";

    public static Uri defaultTreeUri() {
        return DocumentsContract.buildTreeDocumentUri(AUTHORITY, "primary:RSDK/v5");
    }

    public static Uri initialTreeUri() {
        return DocumentsContract.buildTreeDocumentUri(AUTHORITY, "primary:RSDK");
    }

    public static void prepare(Activity activity) {
        ensureDirectStorage();
        if (Build.VERSION.SDK_INT >= 30) {
            if (!Environment.isExternalStorageManager()) {
                try {
                    activity.startActivity(new Intent(
                        Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                        Uri.parse("package:" + activity.getPackageName())));
                } catch (Exception e) {
                    activity.startActivity(new Intent(Settings.ACTION_MANAGE_EXTERNAL_STORAGE_SETTINGS));
                }
            }
        } else {
            ActivityCompat.requestPermissions(activity, new String[] {
                Manifest.permission.READ_EXTERNAL_STORAGE,
                Manifest.permission.WRITE_EXTERNAL_STORAGE
            }, STORAGE_REQUEST);
        }
    }

    public static Uri prepareTree(Context context, Uri selected) {
        if (selected == null) return defaultTreeUri();
        DocumentFile tree = DocumentFile.fromTreeUri(context, selected);
        if (tree == null) return selected;
        DocumentFile v5 = tree;
        if (!DATA_DIR.equals(tree.getName())) {
            DocumentFile existing = tree.findFile(DATA_DIR);
            v5 = existing != null ? existing : tree.createDirectory(DATA_DIR);
        }
        if (v5 == null) return selected;
        DocumentFile mods = v5.findFile("mods");
        if (mods == null) mods = v5.createDirectory("mods");
        writeIfMissing(context, v5, "Settings.ini", SETTINGS_INI);
        if (mods != null) writeIfMissing(context, mods, "modconfig.ini", MODCONFIG_INI);
        try {
            return DocumentsContract.buildTreeDocumentUri(
                selected.getAuthority(), DocumentsContract.getDocumentId(v5.getUri()));
        } catch (Exception e) {
            Log.w("ChaosStorage", "Could not resolve RSDK/v5 tree URI", e);
            return selected;
        }
    }

    private static void ensureDirectStorage() {
        File root = new File(Environment.getExternalStorageDirectory(), RSDK_ROOT);
        File data = new File(root, DATA_DIR);
        File mods = new File(data, "mods");
        root.mkdirs();
        data.mkdirs();
        mods.mkdirs();
        writeIfMissing(new File(data, "Settings.ini"), SETTINGS_INI);
        writeIfMissing(new File(mods, "modconfig.ini"), MODCONFIG_INI);
    }

    private static void writeIfMissing(File file, String contents) {
        if (file.exists()) return;
        try (FileWriter writer = new FileWriter(file)) {
            writer.write(contents);
        } catch (IOException e) {
            Log.w("ChaosStorage", "Could not write " + file.getAbsolutePath(), e);
        }
    }

    private static void writeIfMissing(Context context, DocumentFile dir,
                                       String name, String contents) {
        if (dir.findFile(name) != null) return;
        DocumentFile file = dir.createFile("text/plain", name);
        if (file == null) return;
        try (OutputStream out = context.getContentResolver().openOutputStream(file.getUri())) {
            if (out != null) out.write(contents.getBytes(StandardCharsets.UTF_8));
        } catch (IOException e) {
            Log.w("ChaosStorage", "Could not write " + name, e);
        }
    }
}
JAVA
# Wire the helper into the pinned launcher and engine activities.
sed -i '/^        basePathStore = new File(getFilesDir(), "basePathStore");$/a\        ChaosStorage.prepare(this);' "$LAUNCHER_JAVA"
sed -i 's/^        refreshStore();$/        refreshStore();\n        if (basePath == null) basePath = ChaosStorage.defaultTreeUri();/' "$LAUNCHER_JAVA"
sed -i 's/^                        basePath = result.getData().getData();$/                        basePath = result.getData().getData();\n                        basePath = ChaosStorage.prepareTree(this, basePath);/' "$LAUNCHER_JAVA"
sed -i 's/\.putExtra(DocumentsContract.EXTRA_INITIAL_URI, basePath)/.putExtra(DocumentsContract.EXTRA_INITIAL_URI, ChaosStorage.initialTreeUri())/' "$LAUNCHER_JAVA"
sed -i 's/^                basePath = Launcher.refreshStore();$/                basePath = Launcher.refreshStore();\n                if (basePath == null) basePath = ChaosStorage.defaultTreeUri();/' "$RSDK_JAVA"
echo '::stage::configured shared RSDK/v5 storage and storage permissions'
"####);
    // Upstream defaults to revision 3 and leaves Plus enabled unless these
    // project properties are present. The Gradle project property is passed for
    // compatibility; the patched CMake arguments above are authoritative.
    let plus_property = if disable_plus == "ON" {
        " -PRETRO_DISABLE_PLUS=ON"
    } else {
        ""
    };
    script.push_str(&format!(
        "./gradlew assembleRelease --no-daemon -PABIFILTERS='{abi}'{plus_property} -PRETRO_REVISION=5\n"
    ));
    script.push_str("echo '::phase::packaging'\n");
    script.push_str(r#"echo '::progress::80'
APK=$(find "$GRADLE_WORK" -type f -name '*.apk' -print -quit)
[ -n "$APK" ] || { echo 'Could not locate a built APK' >&2; exit 3; }
cp "$APK" "$OUT/SonicMania.apk"
"#);
}

fn push_macos_script(
    script: &mut String,
    target: BuildTarget,
    disable_plus: &str,
    rsdk_autobuild: &str,
) {
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
        "ARCHS='{architectures}'\nXCODE_BUILD=\"$BUILD/xcode\"\nOTHER_CFLAGS='$(inherited) -DRETRO_REVISION=5 -DRETRO_DISABLE_PLUS={disable_plus} -DRSDK_AUTOBUILD={rsdk_autobuild}'\n"
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
# Data.rsdk stays with the user; an opted-in host-side copy goes beside the .app.
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
            false,
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
            false,
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
            false,
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
        assert!(script.contains("-DRETRO_MOD_LOADER=ON"));
        assert!(script.contains("-DRETRO_MOD_LOADER_VER=2"));
        assert!(script.contains("android:requestLegacyExternalStorage=\"true\""));
        assert!(script.contains("ChaosStorage.java"));
        assert!(script.contains("primary:RSDK/v5"));
        assert!(script.contains("ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION"));
        assert!(script.contains("System.loadLibrary(\"SonicMania\")"));
        assert!(script.contains("android:value=\"SonicMania\""));
        assert!(!script.contains("cp \"$DATA\""));
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
            false,
        );
        assert!(script.contains("xcodebuild"));
        assert!(script.contains("ARCHS='x86_64 arm64'"));
        assert!(!script.contains("cp \"$DATA\""));
    }

    #[test]
    fn plus_opt_in_switches_the_disable_flag_off_for_every_target() {
        let targets = [
            BuildTarget {
                platform: TargetPlatform::Windows,
                architecture: TargetArchitecture::X86_64,
                format: OutputFormat::Exe,
            },
            BuildTarget {
                platform: TargetPlatform::Linux,
                architecture: TargetArchitecture::X86_64,
                format: OutputFormat::Elf,
            },
            BuildTarget {
                platform: TargetPlatform::Android,
                architecture: TargetArchitecture::Aarch64,
                format: OutputFormat::Apk,
            },
            BuildTarget {
                platform: TargetPlatform::Macos,
                architecture: TargetArchitecture::Universal,
                format: OutputFormat::App,
            },
        ];
        for target in targets {
            let script = build_script(target, &paths(), true);
            assert!(
                script.contains("-DRETRO_DISABLE_PLUS=OFF"),
                "Plus opt-in must pass OFF for {target:?}"
            );
            assert!(
                script.contains("-DRETRO_REVISION=5"),
                "revision 5 must stay pinned for {target:?}"
            );
            if matches!(
                target.platform,
                TargetPlatform::Android | TargetPlatform::Macos
            ) {
                assert!(
                    script.contains("-DRSDK_AUTOBUILD=0"),
                    "Plus content must be allowed for {target:?}"
                );
            }
        }

        let default = build_script(
            BuildTarget {
                platform: TargetPlatform::Linux,
                architecture: TargetArchitecture::X86_64,
                format: OutputFormat::Elf,
            },
            &paths(),
            false,
        );
        assert!(default.contains("-DRETRO_DISABLE_PLUS=ON"));
    }
}
