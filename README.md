# ChaosLauncher

ChaosLauncher is a Tauri + Tailwind desktop launcher that compiles Sonic Mania from the pinned [`Sonic-Mania-Decompilation`](https://github.com/RSDKModding/Sonic-Mania-Decompilation) source in an ephemeral Docker container. It provides a three-step workflow: select your own `Data.rsdk`, choose a target, then watch live build logs and progress.

## Legal and licensing

- **ChaosLauncher itself is licensed under the GNU General Public License v3.0.** The complete text is in [`LICENSE`](LICENSE).
- The engine and game source fetched at build time come from **RSDKModding** and remain governed by their own upstream terms, including the **RSDKv5 Decompilation License (strictly non-commercial)**. ChaosLauncher does not replace, relicense, or redistribute that source as part of itself.
- See [`NOTICE`](NOTICE) for the full attribution notice.

ChaosLauncher **never downloads, embeds, or distributes `Data.rsdk` or prebuilt game binaries**, and its own source, release bundles, and AppImage never contain game data. You must supply your own legally obtained copy. The file is selected through a native picker, validated locally, and mounted read-only into the build container solely so the build can read it when needed. By default, generated outputs contain compiled binaries only. Settings includes an opt-in (off by default) to copy your selected `Data.rsdk` into the generated output folder on this host for personal local use; never share, distribute, or upload that output. Your original file is never modified or moved.

## Requirements

- A running Docker daemon for Windows, Linux, and Android builds. The first launch checks for `sonic-mania-builder:latest` and builds it from the embedded `Dockerfile`; later launches reuse the cached image.
- No local compiler toolchain is required for containerized targets.
- macOS `.app` builds require a **native macOS host** with Xcode command-line tools. Apple SDK restrictions make Linux Docker cross-compilation unreliable, so the macOS option is disabled on other hosts and the backend rejects it.
- Building ChaosLauncher from source requires Node.js/npm, Rust/Cargo, and the Tauri CLI.

## Supported targets

| Platform | Architectures | Output formats |
|---|---|---|
| Windows | x86-64 | Standalone `.exe`, portable folder (executable + DLLs only) |
| Linux | x86-64 | ELF binary, `.AppImage` |
| Android | ARM64, ARMv7 | `.apk` |
| macOS | Universal, x86-64, ARM64 | `.app` (native macOS host only) |

ARM64 Windows and Linux targets are intentionally rejected up front because the pinned builder image does not support them. Builds always use `-DRETRO_REVISION=5`. Plus data is disabled by default and can be enabled only through the saved Settings opt-in. Likewise, copying your local `Data.rsdk` into a generated output is off by default and must be opted into in Settings.

## How it works

1. **Source asset** — the native picker resolves a real path to your `Data.rsdk`. Rust validates the header and reports the file count, size, and SHA-256 for identity. The asset is never uploaded or modified.
2. **Target** — choose a platform, architecture, and output format. The UI mirrors the backend's supported combinations and disables unavailable targets.
3. **Build** — the launcher recursively clones the pinned upstream source into its application cache, creates an ephemeral container with the source and asset mounted read-only, compiles the selected target, copies only the chosen artifact(s) to `outputs/<target>-<timestamp>`, and removes the container. If the Settings opt-in is enabled, the launcher then copies your selected `Data.rsdk` from its original host path into that generated output folder (inside the Windows portable folder, or beside other artifacts). It is never embedded in an AppImage, APK, or `.app`. Live `::progress::NN` / `::stage::text` lines are streamed into the progress view, and the activity feed and build summary identify any local data copy. Every container build re-checks the builder image, so a Full Clean that removed it is rebuilt automatically on the next build.

Android builds use a writable copy of the source and the pinned RSDKv5 Gradle project. The pinned checkout omits some upstream Android dependencies, so verified `libogg` and `libtheora` source tarballs are vendored under `vendor/` and staged into the builder image. They are build inputs only and are never placed in the APK.

The Android port follows the local `~/Downloads/Sonic` runtime layout: the generated APK creates and uses `RSDK/v5` at the shared external storage root (for example `/storage/emulated/0/RSDK/v5`) through SAF tree access. The writable Gradle copy is patched to keep the mod loader on (`-DRETRO_MOD_LOADER=ON`, `-DRETRO_MOD_LOADER_VER=2`), request legacy storage access on Android 10 and all-files access on Android 11+, and load the renamed `SonicMania` engine library. It writes reference-compatible `Settings.ini` and `mods/modconfig.ini` files only when they are missing, so your saves, game data, and enabled mods are never replaced. When the local-data opt-in is enabled, the host also stages an `RSDK/v5` folder beside the APK with your `Data.rsdk`; copy that folder to the device yourself. The APK itself never contains game data.

## Development

```sh
npm install
npm run dev      # Vite frontend
npm run tauri dev
npm run build    # TypeScript check + Vite production bundle
```

Rust checks and tests live under `src-tauri/`:

```sh
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

## Vendored build dependencies

| File | Purpose | SHA-256 |
|---|---|---|
| `vendor/libogg-1.3.5.tar.gz` | Android native Ogg decoding source | `0eb4b4b9420a0f51db142ba3f9c64b333f826532dc0f48c6410ae51f4799b664` |
| `vendor/libtheora-1.1.1.tar.gz` | Android native Theora decoding source | `40952956c47811928d1e7922cda3bc1f427eb75680c3c37249c91e949054916b` |

These archives remain under their respective upstream license terms; see the license files inside each archive. Download and build history is recorded in [`dev-downloads.txt`](dev-downloads.txt).

## Credits

Sonic Mania and the decompilation project are credited to **RSDKModding** and its contributors. ChaosLauncher is an independent build frontend and is not affiliated with or endorsed by SEGA or RSDKModding.

See [`NOTICE`](NOTICE) for attribution and license separation details.
