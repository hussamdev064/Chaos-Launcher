import { errorMessage, pickDataRsdk } from "../api";
import { escapeHtml, queryRequired } from "./dom";
import { formatBytes } from "./format";
import type { SourceAsset } from "../types";

const EXPECTED_FILENAME = "data.rsdk";

export interface DropZoneOptions {
  asset: SourceAsset | null;
  onAssetChange: (asset: SourceAsset) => void;
}

/**
 * Step 1 UI. The native picker is used even for drops, because a browser
 * `File` never exposes a real filesystem path that Rust can bind-mount.
 * The file is validated by Rust and is never copied or uploaded.
 */
export function renderDropZone({ asset, onAssetChange }: DropZoneOptions): HTMLElement {
  const section = document.createElement("section");
  section.innerHTML = `
    <div class="mb-5">
      <h2 class="text-2xl font-semibold tracking-tight">Add your game data</h2>
      <p class="mt-1 text-sm text-slate-400">
        Select your own legally obtained <code class="rounded bg-slate-800 px-1.5 py-0.5 text-sky-300">Data.rsdk</code>.
        ChaosLauncher never downloads or bundles game assets.
      </p>
    </div>

    <div data-role="drop-zone" role="button" tabindex="0" aria-label="Choose Data.rsdk"
         class="group flex cursor-pointer flex-col items-center gap-3 rounded-3xl border border-dashed border-slate-700 bg-slate-900/50 px-6 py-14 text-center transition hover:border-sky-500/70 hover:bg-slate-900">
      <svg class="h-10 w-10 text-sky-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true">
        <path d="M12 16V4m0 0L7.5 8.5M12 4l4.5 4.5M4 16v2.5A1.5 1.5 0 0 0 5.5 20h13a1.5 1.5 0 0 0 1.5-1.5V16" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
      <p class="text-lg font-medium">Drop <span class="text-sky-300">Data.rsdk</span> here</p>
      <p class="text-sm text-slate-400">or click to open the native picker — the file stays on your machine</p>
    </div>

    <p data-role="error" class="mt-3 min-h-5 text-sm text-rose-400" aria-live="polite"></p>

    ${
      asset
        ? `<div class="mt-2 flex items-center gap-3 rounded-2xl border border-emerald-500/30 bg-emerald-500/10 p-4">
             <span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-emerald-500/20 text-emerald-300" aria-hidden="true">✓</span>
             <div class="min-w-0">
               <p class="truncate font-medium">${escapeHtml(asset.name)}</p>
               <p class="text-xs text-slate-400">${formatBytes(asset.sizeBytes)} · ${
                 asset.path ? "validated locally" : "awaiting native picker"
               }</p>
               ${
                 asset.sha256
                   ? `<p class="mt-1 truncate font-mono text-[11px] text-slate-500" title="${escapeHtml(asset.sha256)}">SHA-256 ${escapeHtml(asset.sha256.slice(0, 16))}…</p>`
                   : ""
               }
             </div>
           </div>`
        : ""
    }
  `;

  const dropZone = queryRequired<HTMLElement>(section, "[data-role='drop-zone']");
  const errorText = queryRequired<HTMLElement>(section, "[data-role='error']");

  let busy = false;

  const choose = async (): Promise<void> => {
    if (busy) {
      return;
    }
    busy = true;
    dropZone.setAttribute("aria-busy", "true");
    errorText.textContent = "Waiting for the file picker...";

    try {
      const selected = await pickDataRsdk();
      if (!selected) {
        errorText.textContent = "";
        return;
      }
      if (selected.name.toLowerCase() !== EXPECTED_FILENAME) {
        errorText.textContent = `Expected a file named Data.rsdk, got "${selected.name}".`;
        return;
      }
      errorText.textContent = "";
      onAssetChange({
        name: selected.name,
        sizeBytes: selected.sizeBytes,
        path: selected.path,
        sha256: selected.sha256,
        fileCount: selected.fileCount,
      });
    } catch (error) {
      errorText.textContent = errorMessage(error);
    } finally {
      busy = false;
      dropZone.removeAttribute("aria-busy");
    }
  };

  dropZone.addEventListener("click", () => void choose());
  dropZone.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      void choose();
    }
  });

  for (const eventName of ["dragenter", "dragover"] as const) {
    dropZone.addEventListener(eventName, (event) => {
      event.preventDefault();
      dropZone.classList.add("drop-zone-active");
    });
  }
  for (const eventName of ["dragleave", "drop"] as const) {
    dropZone.addEventListener(eventName, (event) => {
      event.preventDefault();
      dropZone.classList.remove("drop-zone-active");
    });
  }

  // A dropped browser File has no usable path, so open the native picker instead.
  dropZone.addEventListener("drop", () => void choose());

  return section;
}
