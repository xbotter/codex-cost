import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

type QuotaMode = "target" | "cap";

type QuotaSettings = {
  enabled: boolean;
  mode: QuotaMode;
  amount_usd: number;
};

let enabledEl: HTMLInputElement | null;
let amountEl: HTMLInputElement | null;
let formEl: HTMLFormElement | null;
let statusEl: HTMLElement | null;
let saveButtonEl: HTMLButtonElement | null;

function setStatus(message: string, isError = false) {
  if (!statusEl) {
    return;
  }

  statusEl.textContent = message;
  statusEl.classList.toggle("is-error", isError);
}

function selectedMode(): QuotaMode {
  const selected = document.querySelector<HTMLInputElement>('input[name="quota-mode"]:checked');
  return selected?.value === "cap" ? "cap" : "target";
}

function applySettings(settings: QuotaSettings) {
  if (!enabledEl || !amountEl) {
    return;
  }

  enabledEl.checked = settings.enabled;
  amountEl.value = settings.amount_usd > 0 ? settings.amount_usd.toFixed(2) : "";

  const targetRadio = document.querySelector<HTMLInputElement>('input[name="quota-mode"][value="target"]');
  const capRadio = document.querySelector<HTMLInputElement>('input[name="quota-mode"][value="cap"]');
  if (targetRadio && capRadio) {
    targetRadio.checked = settings.mode === "target";
    capRadio.checked = settings.mode === "cap";
  }
}

async function loadSettings() {
  const settings = await invoke<QuotaSettings>("get_quota_settings");
  applySettings(settings);
  setStatus("");
}

async function saveSettings(event: SubmitEvent) {
  event.preventDefault();

  if (!enabledEl || !amountEl || !saveButtonEl) {
    return;
  }

  const parsed = Number.parseFloat(amountEl.value);
  const settings: QuotaSettings = {
    enabled: enabledEl.checked,
    mode: selectedMode(),
    amount_usd: Number.isFinite(parsed) ? parsed : 0,
  };

  if (settings.enabled && (!Number.isFinite(settings.amount_usd) || settings.amount_usd <= 0)) {
    setStatus("Enter a valid positive USD amount.", true);
    return;
  }

  saveButtonEl.disabled = true;
  setStatus("Saving...");

  try {
    const saved = await invoke<QuotaSettings>("save_quota_settings", { settings });
    applySettings(saved);
    setStatus("Saved");
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setStatus(message, true);
  } finally {
    saveButtonEl.disabled = false;
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  formEl = document.querySelector("#quota-form");
  enabledEl = document.querySelector("#quota-enabled");
  amountEl = document.querySelector("#quota-amount");
  statusEl = document.querySelector("#form-status");
  saveButtonEl = document.querySelector("#save-button");

  formEl?.addEventListener("submit", (event) => void saveSettings(event));

  await loadSettings();

  await listen("settings-window-opened", async () => {
    await loadSettings();
  });

  const currentWindow = getCurrentWindow();
  currentWindow.onCloseRequested(async (event) => {
    event.preventDefault();
    setStatus("");
    await loadSettings();
    await currentWindow.hide();
  });
});
