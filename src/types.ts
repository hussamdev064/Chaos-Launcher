export type TargetPlatform = "windows" | "linux" | "android" | "macos";
export type TargetArchitecture = "x86_64" | "aarch64" | "armv7" | "universal";
export type OutputFormat = "exe" | "portable" | "elf" | "appimage" | "apk" | "app";

/** The user's own legally obtained asset. This app never fetches or embeds one. */
export interface SourceAsset {
  name: string;
  sizeBytes: number;
  /** Real filesystem path supplied by Tauri's native picker; required for builds. */
  path: string | null;
  sha256?: string;
  fileCount?: number;
}

export interface BuildTargetSelection {
  platform: TargetPlatform;
  architecture: TargetArchitecture;
  format: OutputFormat;
}

export interface BuildSelection extends BuildTargetSelection {
  asset: SourceAsset | null;
}

export type StepIndex = 0 | 1 | 2;
