import { queryRequired } from "./dom";
import type { OutputFormat, TargetArchitecture, TargetPlatform } from "../types";

export interface FormatOption {
  id: OutputFormat;
  label: string;
  hint: string;
}

export interface PlatformOption {
  id: TargetPlatform;
  label: string;
  hint: string;
  architectures: TargetArchitecture[];
  formats: FormatOption[];
  requiresMacHost: boolean;
}

const ARCHITECTURE_LABELS: Record<TargetArchitecture, string> = {
  x86_64: "x86-64",
  aarch64: "ARM64",
  armv7: "ARMv7",
  universal: "Universal",
};

/** UI mirror of supported combinations; Rust validates every build again before it starts. */
export const PLATFORM_OPTIONS: readonly PlatformOption[] = [
  {
    id: "windows",
    label: "Windows",
    hint: "MinGW cross-build",
    architectures: ["x86_64"],
    formats: [
      { id: "exe", label: "Standalone .exe", hint: "Single executable" },
      { id: "portable", label: "Portable folder", hint: "Executable + DLLs (no game data)" },
    ],
    requiresMacHost: false,
  },
  {
    id: "linux",
    label: "Linux",
    hint: "SDL2 / GLEW build",
    architectures: ["x86_64"],
    formats: [
      { id: "elf", label: "ELF binary", hint: "Plain executable" },
      { id: "appimage", label: ".AppImage", hint: "Portable Linux bundle" },
    ],
    requiresMacHost: false,
  },
  {
    id: "android",
    label: "Android",
    hint: "Gradle / NDK build",
    architectures: ["aarch64", "armv7"],
    formats: [{ id: "apk", label: ".apk", hint: "Installable package" }],
    requiresMacHost: false,
  },
  {
    id: "macos",
    label: "macOS",
    hint: "Native host only",
    architectures: ["universal", "x86_64", "aarch64"],
    formats: [{ id: "app", label: ".app bundle", hint: "Native macOS bundle" }],
    requiresMacHost: true,
  },
];

export function platformOption(id: TargetPlatform): PlatformOption {
  const option = PLATFORM_OPTIONS.find((candidate) => candidate.id === id);
  if (!option) {
    throw new Error(`Unknown target platform: ${id}`);
  }
  return option;
}

export function formatOption(platform: TargetPlatform, format: OutputFormat): FormatOption | undefined {
  return platformOption(platform).formats.find((candidate) => candidate.id === format);
}

/**
 * Fallback when the Tauri host-platform command is unavailable (e.g. plain
 * browser preview). In the packaged app the backend's `std::env::consts::OS`
 * value is authoritative.
 */
export function isMacHost(): boolean {
  return /Mac/i.test(navigator.userAgent);
}

export function defaultsForPlatform(id: TargetPlatform): {
  architecture: TargetArchitecture;
  format: OutputFormat;
} {
  const option = platformOption(id);
  return {
    architecture: option.architectures[0],
    format: option.formats[0].id,
  };
}

export interface TargetSelectorOptions {
  platform: TargetPlatform;
  architecture: TargetArchitecture;
  format: OutputFormat;
  /** Authoritative host OS from Rust; falls back to user-agent detection. */
  isMacHost?: boolean;
  /** `compact` is used by the dashboard; `wizard` keeps the original step view. */
  variant?: "wizard" | "compact";
  onChange: (platform: TargetPlatform, architecture: TargetArchitecture, format: OutputFormat) => void;
}

export function renderTargetSelector({
  platform,
  architecture,
  format,
  isMacHost: macHostOverride,
  variant = "wizard",
  onChange,
}: TargetSelectorOptions): HTMLElement {
  const current = platformOption(platform);
  const macHost = macHostOverride ?? isMacHost();
  const compact = variant === "compact";
  const section = document.createElement("section");

  section.innerHTML = `
    <div class="${compact ? "mb-3" : "mb-5"}">
      <h2 class="${
        compact
          ? "text-base font-semibold text-slate-200"
          : "text-2xl font-semibold tracking-tight"
      }">${compact ? "Target profile" : "Choose your target"}</h2>
      <p class="${
        compact ? "mt-0.5 text-[11px] text-slate-500" : "mt-1 text-sm text-slate-400"
      }">${
        compact
          ? "Platform, architecture, and output format"
          : "Platform, CPU architecture, and output format."
      }</p>
    </div>

    <div class="${compact ? "grid grid-cols-2 gap-2 sm:grid-cols-4" : "grid gap-3 sm:grid-cols-2"}" data-role="platforms">
      ${PLATFORM_OPTIONS.map(
        (option) => {
          const disabled = option.requiresMacHost && !macHost;
          const selected = option.id === platform;
          return `
            <button type="button" data-platform="${option.id}"
                    ${disabled ? 'disabled aria-disabled="true"' : `aria-pressed="${selected}"`}
                    class="rounded-xl border ${
                      compact ? "p-2.5" : "p-4"
                    } text-left transition ${
                      selected
                        ? "border-sky-500 bg-sky-500/10"
                        : "border-slate-800 bg-slate-900/50 hover:border-slate-600"
                    } ${disabled ? "cursor-not-allowed opacity-40" : ""}">
              <span class="${compact ? "text-sm" : ""} block font-medium">${option.label}</span>
              <span class="mt-1 block text-xs text-slate-400">${
                disabled ? "Requires a macOS host" : compact ? option.hint.split(" ")[0] : option.hint
              }</span>
            </button>`;
        },
      ).join("")}
    </div>

    <div class="${compact ? "mt-4" : "mt-7"}">
      <h3 class="mb-2 text-xs font-semibold uppercase tracking-widest text-slate-500">Architecture</h3>
      <div class="flex flex-wrap gap-2" data-role="architectures">
        ${current.architectures
          .map(
            (option) => `
              <button type="button" data-architecture="${option}" aria-pressed="${option === architecture}"
                      class="rounded-lg border ${
                        compact ? "px-3 py-1.5 text-xs" : "px-4 py-2 text-sm"
                      } transition ${
                        option === architecture
                          ? "border-sky-500 bg-sky-500/10 text-sky-200"
                          : "border-slate-800 bg-slate-900/50 text-slate-300 hover:border-slate-600"
                      }">${ARCHITECTURE_LABELS[option]}</button>`,
          )
          .join("")}
      </div>
    </div>

    <div class="${compact ? "mt-4" : "mt-7"}">
      <h3 class="mb-2 text-xs font-semibold uppercase tracking-widest text-slate-500">Output format</h3>
      <div class="${compact ? "grid gap-2 sm:grid-cols-2" : "grid gap-3 sm:grid-cols-2"}" data-role="formats">
        ${current.formats
          .map(
            (option) => `
              <button type="button" data-format="${option.id}" aria-pressed="${option.id === format}"
                      class="rounded-xl border ${
                        compact ? "p-2.5" : "p-4"
                      } text-left transition ${
                        option.id === format
                          ? "border-sky-500 bg-sky-500/10"
                          : "border-slate-800 bg-slate-900/50 hover:border-slate-600"
                      }">
                <span class="${compact ? "text-xs" : ""} block font-medium">${option.label}</span>
                <span class="mt-0.5 block text-xs text-slate-400">${
                  compact ? option.hint.split(" ")[0] : option.hint
                }</span>
              </button>`,
          )
          .join("")}
      </div>
    </div>

    ${
      macHost
        ? ""
        : `<p class="${
            compact ? "mt-4" : "mt-6"
          } rounded-xl border border-amber-500/25 bg-amber-500/10 p-3 text-xs text-amber-200">
             macOS builds require a macOS host: Apple SDK restrictions make Linux container cross-compilation unreliable.
           </p>`
    }
  `;

  section.querySelectorAll<HTMLButtonElement>("[data-platform]").forEach((button) => {
    button.addEventListener("click", () => {
      if (button.disabled) {
        return;
      }
      const nextPlatform = button.dataset.platform as TargetPlatform;
      const defaults = defaultsForPlatform(nextPlatform);
      onChange(nextPlatform, defaults.architecture, defaults.format);
    });
  });

  section.querySelectorAll<HTMLButtonElement>("[data-architecture]").forEach((button) => {
    button.addEventListener("click", () => {
      onChange(platform, button.dataset.architecture as TargetArchitecture, format);
    });
  });

  section.querySelectorAll<HTMLButtonElement>("[data-format]").forEach((button) => {
    button.addEventListener("click", () => {
      onChange(platform, architecture, button.dataset.format as OutputFormat);
    });
  });

  // Keep the section heading reachable for assistive tech after a re-render.
  queryRequired<HTMLElement>(section, "h2").setAttribute("tabindex", "-1");
  return section;
}
