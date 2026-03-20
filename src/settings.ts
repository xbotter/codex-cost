import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

type QuotaMode = "target" | "cap";

type QuotaSettings = {
  enabled: boolean;
  mode: QuotaMode;
  amount_usd: number;
};

type DashboardSettings = {
  always_on_top: boolean;
  current_provider: string;
  enabled_providers: string[];
};

let enabledEl: HTMLInputElement | null;
let amountEl: HTMLInputElement | null;
let formEl: HTMLFormElement | null;
let statusEl: HTMLElement | null;
let saveButtonEl: HTMLButtonElement | null;
let providerCodexEl: HTMLInputElement | null;
let providerClaudeEl: HTMLInputElement | null;
let tabButtons: HTMLButtonElement[] = [];
let tabPanels: HTMLElement[] = [];
let currentDashboardSettings: DashboardSettings | null = null;
let currentTab: "quota" | "providers" = "quota";

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

function applyDashboardSettings(settings: DashboardSettings) {
  currentDashboardSettings = settings;
  if (providerCodexEl) {
    providerCodexEl.checked = settings.enabled_providers.includes("codex");
  }
  if (providerClaudeEl) {
    providerClaudeEl.checked = settings.enabled_providers.includes("claude");
  }
}

function setActiveTab(nextTab: "quota" | "providers") {
  currentTab = nextTab;

  tabButtons.forEach((button) => {
    const isActive = button.dataset.tab === nextTab;
    button.classList.toggle("is-active", isActive);
    button.setAttribute("aria-selected", isActive ? "true" : "false");
  });

  tabPanels.forEach((panel) => {
    const isActive = panel.id === `panel-${nextTab}`;
    panel.classList.toggle("is-hidden", !isActive);
    panel.hidden = !isActive;
  });
}

async function loadSettings() {
  const [quotaSettings, dashboardSettings] = await Promise.all([
    invoke<QuotaSettings>("get_quota_settings"),
    invoke<DashboardSettings>("get_dashboard_settings"),
  ]);
  applySettings(quotaSettings);
  applyDashboardSettings(dashboardSettings);
  setStatus("");
}

async function saveSettings(event: SubmitEvent) {
  event.preventDefault();

  if (!enabledEl || !amountEl || !saveButtonEl || !providerCodexEl || !providerClaudeEl) {
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

  const enabledProviders = [
    providerCodexEl.checked ? "codex" : null,
    providerClaudeEl.checked ? "claude" : null,
  ].filter((provider): provider is string => Boolean(provider));

  if (!enabledProviders.length) {
    setStatus("Keep at least one provider enabled.", true);
    return;
  }

  const dashboardSettings: DashboardSettings = {
    always_on_top: currentDashboardSettings?.always_on_top ?? false,
    current_provider: currentDashboardSettings?.current_provider ?? enabledProviders[0],
    enabled_providers: enabledProviders,
  };

  saveButtonEl.disabled = true;
  setStatus("Saving...");

  try {
    const [savedQuota, savedDashboard] = await Promise.all([
      invoke<QuotaSettings>("save_quota_settings", { settings }),
      invoke<DashboardSettings>("save_dashboard_settings", { settings: dashboardSettings }),
    ]);
    applySettings(savedQuota);
    applyDashboardSettings(savedDashboard);
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
  providerCodexEl = document.querySelector("#provider-codex");
  providerClaudeEl = document.querySelector("#provider-claude");
  tabButtons = Array.from(document.querySelectorAll<HTMLButtonElement>(".settings-tab"));
  tabPanels = Array.from(document.querySelectorAll<HTMLElement>(".settings-panel-section"));

  formEl?.addEventListener("submit", (event) => void saveSettings(event));
  tabButtons.forEach((button) => {
    const tabName = button.dataset.tab === "providers" ? "providers" : "quota";
    button.addEventListener("click", () => {
      setActiveTab(tabName);
    });
  });

  await loadSettings();
  setActiveTab(currentTab);

  await listen("settings-window-opened", async () => {
    await loadSettings();
    setActiveTab(currentTab);
  });

  const currentWindow = getCurrentWindow();
  currentWindow.onCloseRequested(async (event) => {
    event.preventDefault();
    setStatus("");
    await loadSettings();
    await currentWindow.hide();
  });
});
