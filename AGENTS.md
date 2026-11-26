# ChaosLauncher — agent notes

Tauri v2 + Tailwind desktop launcher that builds Sonic Mania from `Sonic-Mania-Decompilation` inside Docker. Phases 1-5 are written (Vite + Tailwind UI in `src/`, Tauri command bridge + Docker/source/asset/build backend in `src-tauri/`, multi-stage `Dockerfile`) but unverified: this host has no Node.js/npm/Rust/Cargo/Docker and no dependencies have been downloaded (see `dev-downloads.txt`). Ask before installing anything. `Prompt.md` is a read-only copy of the original project spec.

## Workflow rules (before any code)
- One phase at a time, in the order below. Plan → implement fully → self-check → stop with a summary and a ready-to-run next-task prompt; wait for approval.
- Ask before downloading/installing any tool, package, SDK, or dependency; record each download in `dev-downloads.txt`.
- Never `git push` without explicit approval.
- Favor `Result`/`Option` over panics; comment Docker mounts and CMake execution inline (explicit project requirement).

## Hard constraints
- Never bundle, download, or distribute `Data.rsdk` or prebuilt game binaries. The user supplies their own legally obtained copy; builds only read it locally.
- No local toolchains for users: compile in ephemeral Docker containers, extract the artifact, clean up.
- Keep `-DRETRO_REVISION=5` and `-DRETRO_DISABLE_PLUS=ON` unless the user explicitly opts in with legitimate Plus data.
- Launcher is GPLv3 (full `LICENSE`); `README.md` + `NOTICE` must credit RSDKModding and state the fetched engine source remains under its own non-commercial RSDKv5 Decompilation License.
- macOS `.app` cannot reliably be cross-compiled from Linux Docker (Apple SDK restrictions). Native build on macOS hosts only; disable or explain the target on Windows/Linux.

## Pinned stack and config (exact values)
- Rust: `tokio 1` (full), `serde 1` (derive), `serde_json 1`, `bollard 0.17`, `git2 0.19`, `tauri 2`; plus `tar 0.4`/`futures-util 0.3` for the Docker context and stream, `sha2 0.10` for asset fingerprints; plugins `dialog`, `fs`, `shell`.
- `capabilities/default.json`: FS read/write for `Data.rsdk` selection and output binaries, plus shell/process permission for opening output directories.
- Docker image `sonic-mania-builder:latest`: check on startup, build from the project `Dockerfile` if missing. Multi-stage; must include build-essential, cmake, ninja, git, wget; SDL2/GLEW/ogg/vorbis dev libs; appimagetool; mingw-w64; Android NDK/SDK + Gradle. `Dockerfile` and `docker/mingw-x86_64.cmake` are embedded via `include_str!` so packaged builds need no source checkout.
- Startup emits `docker://status` with `{ stage, message }` (`checking` / `building` / `ready` / `failed`); the UI caches it in Tauri state, listens live, and fetches `docker_status` on mount so events emitted before listener setup are not lost.
- Linux: CMake with `-DRETRO_SUBSYSTEM=SDL2 -DUSE_SDL_AUDIO=ON`, linking SDL2, GLEW, libogg, libvorbis. Windows: MinGW via `-DCMAKE_TOOLCHAIN_FILE=/opt/cmake/mingw-x86_64.cmake` (default DX9). Upstream binary is renamed with `-DRETRO_OUTPUT_NAME=SonicMania`.
- Android: `dependencies/RSDKv5/android/gradlew assembleRelease -PABIFILTERS=...` on a writable copy; the source mount stays read-only. macOS: `xcodebuild` against `SonicMania.xcodeproj` and `dependencies/RSDKv5/RSDKv5.xcodeproj` (the pinned engine has no Darwin CMake platform).
- Source clone: `https://github.com/RSDKModding/Sonic-Mania-Decompilation.git` into `app_cache_dir()/Sonic-Mania-Decompilation`; it **must** be recursive (`dependencies/RSDKv5` is a submodule). Never use a GitHub ZIP.
- `Data.rsdk` header (verified upstream): bytes `RSDK` + `v` + `5`, then u16 LE file count, then 24-byte entries. No expected hash is bundled; SHA-256 is computed only for logging or an optional user-supplied comparison.
- Containers mount source + `Data.rsdk` read-only at `/work/source` and `/work/Data.rsdk`, output at `/work/out:rw`; `/work/build` is disposable. Containers are removed on every path.
- Image only cross-compiles Windows/Linux **x86-64** and Android arm64/armv7; ARM64 Windows/Linux is rejected up front. macOS `.app` runs natively on macOS hosts only (`cfg!(target_os = "macos")`), never in Docker.
- Build scripts emit `::progress::NN` and `::stage::text` lines on stdout; the runner parses them into `BuildEvent`s, and the command bridge re-emits them on `build://event` as `{ percent, message }`.
- Packaged outputs never include `Data.rsdk`: portable folders contain binaries/DLLs only, AppImage/.app use the user's local asset. The mount exists so builds can read it if needed.
- Tauri commands: `docker_status`, `host_platform`, `choose_data_rsdk`, `validate_asset`, `prepare_source`, `start_build`, `open_output_folder`. Native picker resolves a real path; Rust validates it and owns `app_cache_dir()/outputs/<target>-<timestamp>`. A managed `Arc<BuildState>` mutex rejects overlapping builds.

## Phase order
1. Scaffold Tauri v2 + Tailwind; 3-step UI: `Data.rsdk` drop zone → platform/architecture/format cards (Windows `.exe` or portable folder, Linux ELF or `.AppImage`, Android `.apk`, macOS `.app`) → progress bar, collapsible logs, "Open Output Folder" / "Compile for Another Platform".
2. Multi-stage `Dockerfile` plus startup image verify/build.
3. `git2` clone with submodules into an app cache directory; validate the user's `Data.rsdk`.
4. `bollard`/`tokio::process` container run: mount source + `Data.rsdk`, build per target, package and extract the chosen format, clean up.
5. Tauri async commands stream live logs to the progress view; wire the opener to the output folder; graceful error recovery.
