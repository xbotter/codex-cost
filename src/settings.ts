import { invoke } from "@tauri-apps/api/core";

type ProviderId = "codex" | "claude" | "kimi";
type QuotaMode = "target" | "cap";
type LimitMode = "none" | QuotaMode;

type QuotaSettings = {
  enabled: boolean;
  mode: QuotaMode;
  amount_usd: number;
};

type ProviderQuotaSettings = Record<ProviderId, QuotaSettings>;

type DashboardSettings = {
  always_on_top: boolean;
  current_provider: string;
  enabled_providers: string[];
};

type ProviderSettingsSummary = {
  id: ProviderId;
  display_name: string;
  description: string;
  status_label: string;
  has_local_data: boolean;
};

const PROVIDER_IDS: ProviderId[] = ["codex", "claude", "kimi"];

let formEl: HTMLFormElement | null;
let providerListEl: HTMLElement | null;
let providerDetailEl: HTMLElement | null;
let statusEl: HTMLElement | null;
let saveButtonEl: HTMLButtonElement | null;

let currentDashboardSettings: DashboardSettings | null = null;
let currentQuotaSettings: ProviderQuotaSettings | null = null;
let providerSummaries: ProviderSettingsSummary[] = [];
let selectedProviderId: ProviderId = "codex";
let initialized = false;

function defaultQuotaSettings(): QuotaSettings {
  return {
    enabled: false,
    mode: "target",
    amount_usd: 0,
  };
}

function normalizeProviderQuotaSettings(settings: Partial<Record<ProviderId, QuotaSettings>>): ProviderQuotaSettings {
  return {
    codex: settings.codex ?? defaultQuotaSettings(),
    claude: settings.claude ?? defaultQuotaSettings(),
    kimi: settings.kimi ?? defaultQuotaSettings(),
  };
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function providerSummary(providerId: ProviderId): ProviderSettingsSummary | undefined {
  return providerSummaries.find((summary) => summary.id === providerId);
}

function compactStatusLabel(summary?: ProviderSettingsSummary): string {
  return summary?.has_local_data ? "Detected" : "Missing";
}

function isProviderEnabled(providerId: ProviderId): boolean {
  return currentDashboardSettings?.enabled_providers.includes(providerId) ?? false;
}

function setStatus(message: string, isError = false) {
  if (!statusEl) {
    return;
  }

  statusEl.textContent = message;
  statusEl.classList.toggle("is-error", isError);
}

function limitModeForQuota(settings: QuotaSettings): LimitMode {
  return settings.enabled ? settings.mode : "none";
}

function validateQuotaSettings(
  settings: ProviderQuotaSettings,
  enabledProviders: ProviderId[],
): string | null {
  for (const providerId of enabledProviders) {
    const quota = settings[providerId];
    if (quota.enabled && (!Number.isFinite(quota.amount_usd) || quota.amount_usd <= 0)) {
      const label = providerSummary(providerId)?.display_name ?? providerId;
      return `Enter a valid positive USD amount for ${label}.`;
    }
  }
  return null;
}

function renderProviderList() {
  if (!providerListEl) {
    return;
  }

  providerListEl.innerHTML = providerSummaries
    .map((summary) => {
      const isActive = summary.id === selectedProviderId;
      const isEnabled = isProviderEnabled(summary.id);
      const statusClass = summary.has_local_data ? "provider-status is-detected" : "provider-status";
      return `
        <button
          class="provider-item${isActive ? " is-active" : ""}"
          type="button"
          data-provider-select="${summary.id}"
        >
          <span class="provider-item-copy">
            <strong>${escapeHtml(summary.display_name)}</strong>
          </span>
          <span class="provider-item-meta">
            <span class="${statusClass}">${compactStatusLabel(summary)}</span>
            <span class="provider-enabled-indicator${isEnabled ? " is-enabled" : ""}"></span>
          </span>
        </button>
      `;
    })
    .join("");

  providerListEl.querySelectorAll<HTMLButtonElement>("[data-provider-select]").forEach((button) => {
    button.addEventListener("click", () => {
      const providerId = button.dataset.providerSelect as ProviderId;
      selectedProviderId = providerId;
      renderProviderList();
      renderProviderDetail();
      setStatus("");
    });
  });
}

function renderProviderDetail() {
  if (!providerDetailEl || !currentDashboardSettings || !currentQuotaSettings) {
    return;
  }

  const summary = providerSummary(selectedProviderId);
  const quota = currentQuotaSettings[selectedProviderId];
  const isEnabled = isProviderEnabled(selectedProviderId);
  const limitMode = limitModeForQuota(quota);
  const amountValue = quota.amount_usd > 0 ? quota.amount_usd.toFixed(2) : "";
  const statusChipClass = summary?.has_local_data ? "provider-status is-detected" : "provider-status";
  const statusLabel = compactStatusLabel(summary);

  providerDetailEl.innerHTML = `
    <div class="detail-header">
      <div class="detail-title">
        <h2>${escapeHtml(summary?.display_name ?? selectedProviderId)}</h2>
        <div class="detail-meta">
          <span class="${statusChipClass}">${escapeHtml(statusLabel)}</span>
        </div>
      </div>
      <label class="detail-toggle">
        <span>Enabled</span>
        <input id="provider-enabled" class="toggle" type="checkbox" ${isEnabled ? "checked" : ""} />
      </label>
    </div>

    <section class="detail-section">
      <h3>Overview</h3>
      <div class="kv-list">
        <div class="kv-row">
          <span class="kv-label">Data</span>
          <span class="kv-value">${escapeHtml(statusLabel)}</span>
        </div>
        <div class="kv-row">
          <span class="kv-label">Dashboard</span>
          <span class="kv-value">${isEnabled ? "Yes" : "No"}</span>
        </div>
      </div>
    </section>

    <section class="detail-section">
      <h3>Daily limit</h3>
      <div class="limit-mode-group" role="radiogroup" aria-label="${escapeHtml(summary?.display_name ?? selectedProviderId)} daily limit mode">
        ${(["none", "target", "cap"] as const)
          .map((mode) => {
            const labels: Record<LimitMode, string> = {
              none: "No limit",
              target: "Target",
              cap: "Cap",
            };
            return `
              <label class="mode-chip">
                <input
                  type="radio"
                  name="quota-mode"
                  value="${mode}"
                  ${limitMode === mode ? "checked" : ""}
                />
                <span>${labels[mode]}</span>
              </label>
            `;
          })
          .join("")}
      </div>
      <label class="amount-field">
        <span>USD / day</span>
        <input
          id="quota-amount"
          class="text-input"
          type="number"
          inputmode="decimal"
          min="0.01"
          step="0.01"
          placeholder="200.00"
          value="${amountValue}"
          ${limitMode === "none" ? "disabled" : ""}
        />
      </label>
    </section>
  `;

  const enabledInput = providerDetailEl.querySelector<HTMLInputElement>("#provider-enabled");
  enabledInput?.addEventListener("change", () => {
    if (!currentDashboardSettings || !enabledInput) {
      return;
    }

    const enabledProviders = currentDashboardSettings.enabled_providers.filter(
      (provider): provider is ProviderId => PROVIDER_IDS.includes(provider as ProviderId),
    );

    if (!enabledInput.checked && enabledProviders.length === 1 && enabledProviders[0] === selectedProviderId) {
      enabledInput.checked = true;
      setStatus("Keep at least one provider enabled.", true);
      return;
    }

    currentDashboardSettings.enabled_providers = enabledInput.checked
      ? Array.from(new Set([...enabledProviders, selectedProviderId]))
      : enabledProviders.filter((provider) => provider !== selectedProviderId);

    if (!currentDashboardSettings.enabled_providers.includes(currentDashboardSettings.current_provider)) {
      currentDashboardSettings.current_provider = currentDashboardSettings.enabled_providers[0] ?? "codex";
    }

    renderProviderList();
    renderProviderDetail();
    setStatus("");
  });

  providerDetailEl.querySelectorAll<HTMLInputElement>('input[name="quota-mode"]').forEach((input) => {
    input.addEventListener("change", () => {
      if (!currentQuotaSettings) {
        return;
      }
      const nextMode = input.value as LimitMode;
      currentQuotaSettings[selectedProviderId] = {
        ...currentQuotaSettings[selectedProviderId],
        enabled: nextMode !== "none",
        mode: nextMode === "cap" ? "cap" : "target",
      };
      renderProviderDetail();
      setStatus("");
    });
  });

  const amountInput = providerDetailEl.querySelector<HTMLInputElement>("#quota-amount");
  amountInput?.addEventListener("input", () => {
    if (!currentQuotaSettings || !amountInput) {
      return;
    }
    const parsed = Number.parseFloat(amountInput.value);
    currentQuotaSettings[selectedProviderId] = {
      ...currentQuotaSettings[selectedProviderId],
      amount_usd: Number.isFinite(parsed) ? parsed : 0,
    };
    setStatus("");
  });

}

async function loadSettings() {
  const [quotaSettings, dashboardSettings, summaries] = await Promise.all([
    invoke<ProviderQuotaSettings>("get_provider_quota_settings"),
    invoke<DashboardSettings>("get_dashboard_settings"),
    invoke<ProviderSettingsSummary[]>("get_provider_settings_summaries"),
  ]);

  currentQuotaSettings = normalizeProviderQuotaSettings(quotaSettings);
  currentDashboardSettings = dashboardSettings;
  providerSummaries = summaries;

  const preferredProvider = dashboardSettings.current_provider as ProviderId;
  selectedProviderId = PROVIDER_IDS.includes(preferredProvider) ? preferredProvider : "codex";

  renderProviderList();
  renderProviderDetail();
  setStatus("");
}

async function saveSettings(event: SubmitEvent) {
  event.preventDefault();

  if (!saveButtonEl || !currentDashboardSettings || !currentQuotaSettings) {
    return;
  }

  const enabledProviders = currentDashboardSettings.enabled_providers.filter(
    (provider): provider is ProviderId => PROVIDER_IDS.includes(provider as ProviderId),
  );

  if (!enabledProviders.length) {
    setStatus("Keep at least one provider enabled.", true);
    return;
  }

  const validationError = validateQuotaSettings(currentQuotaSettings, enabledProviders);
  if (validationError) {
    setStatus(validationError, true);
    return;
  }

  const dashboardSettings: DashboardSettings = {
    always_on_top: currentDashboardSettings.always_on_top,
    current_provider: enabledProviders.includes(currentDashboardSettings.current_provider as ProviderId)
      ? currentDashboardSettings.current_provider
      : enabledProviders[0],
    enabled_providers: enabledProviders,
  };

  saveButtonEl.disabled = true;
  setStatus("Saving...");

  try {
    const [savedQuotaSettings, savedDashboard] = await Promise.all([
      invoke<ProviderQuotaSettings>("save_provider_quota_settings", { settings: currentQuotaSettings }),
      invoke<DashboardSettings>("save_dashboard_settings", { settings: dashboardSettings }),
    ]);
    currentQuotaSettings = normalizeProviderQuotaSettings(savedQuotaSettings);
    currentDashboardSettings = savedDashboard;
    renderProviderList();
    renderProviderDetail();
    setStatus("Saved");
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setStatus(message, true);
  } finally {
    saveButtonEl.disabled = false;
  }
}

export function initializeSettingsView() {
  if (initialized) {
    return;
  }

  formEl = document.querySelector("#settings-form");
  providerListEl = document.querySelector("#provider-list");
  providerDetailEl = document.querySelector("#provider-detail");
  statusEl = document.querySelector("#form-status");
  saveButtonEl = document.querySelector("#save-button");

  formEl?.addEventListener("submit", (event) => void saveSettings(event));
  initialized = true;
}

export async function loadSettingsView() {
  if (!initialized) {
    initializeSettingsView();
  }

  await loadSettings();
}
