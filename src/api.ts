import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { OutputFormat, TargetArchitecture, TargetPlatform } from "./types";

export const BUILD_EVENT = "build://event";
export const DOCKER_STATUS_EVENT = "docker://status";

export type DockerStage = "checking" | "building" | "ready" | "failed";
export type Unlisten = () => void;

export interface DockerStatus {
  stage: DockerStage;
  message: string;
}

export interface ValidatedAsset {
  name: string;
  path: string;
  sizeBytes: number;
  sha256: string;
  fileCount: number;
}

export interface BuildTargetDto {
  platform: TargetPlatform;
  architecture: TargetArchitecture;
  format: OutputFormat;
}

export interface StartBuildPayload {
  assetPath: string;
  target: BuildTargetDto;
}

export interface BuildEventPayload {
  percent: number | null;
  message: string;
}

export interface BuildResponse {
  outputDirectory: string;
  artifacts: string[];
  assetSha256: string;
}

/** Tauri command errors arrive as strings; normalize anything else for display. */
export function errorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  try {
    return JSON.stringify(error) ?? String(error);
  } catch {
    return String(error);
  }
}

export async function fetchDockerStatus(): Promise<DockerStatus> {
  return invoke<DockerStatus>("docker_status");
}

export async function fetchHostPlatform(): Promise<string> {
  return invoke<string>("host_platform");
}

/** Opens the native picker; resolves to null when the user cancels. */
export async function pickDataRsdk(): Promise<ValidatedAsset | null> {
  return invoke<ValidatedAsset | null>("choose_data_rsdk");
}

export async function validateAsset(path: string): Promise<ValidatedAsset> {
  return invoke<ValidatedAsset>("validate_asset", { path });
}

export async function prepareSource(): Promise<string> {
  return invoke<string>("prepare_source");
}

export async function startBuild(payload: StartBuildPayload): Promise<BuildResponse> {
  return invoke<BuildResponse>("start_build", { request: payload });
}

export async function openOutputFolder(path: string): Promise<void> {
  await invoke<void>("open_output_folder", { path });
}

export function listenDockerStatus(handler: (status: DockerStatus) => void): Promise<Unlisten> {
  return listen<DockerStatus>(DOCKER_STATUS_EVENT, (event) => handler(event.payload));
}

export function listenBuildEvents(
  handler: (event: BuildEventPayload) => void,
): Promise<Unlisten> {
  return listen<BuildEventPayload>(BUILD_EVENT, (event) => handler(event.payload));
}
