import "@fontsource/nunito/latin-400.css";
import "@fontsource/nunito/latin-600.css";
import "@fontsource/nunito/latin-700.css";
import "@fontsource/nunito/latin-800.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { toPng } from "html-to-image";
import arrowLeftIcon from "lucide-static/icons/arrow-left.svg?raw";
import pinIcon from "lucide-static/icons/pin.svg?raw";
import pinOffIcon from "lucide-static/icons/pin-off.svg?raw";
import settingsIcon from "lucide-static/icons/settings-2.svg?raw";
import shareIcon from "lucide-static/icons/share.svg?raw";
import calendarIcon from "lucide-static/icons/calendar-range.svg?raw";
import loaderIcon from "lucide-static/icons/loader-circle.svg?raw";
import { initializeSettingsView, loadSettingsView } from "./settings";

type TokenUsage = {
  input_tokens: number;
  cached_input_tokens: number;
  cache_creation_input_tokens: number;
  output_tokens: number;
  reasoning_output_tokens: number;
};

type CostBreakdown = {
  model_name: string;
  normalized_model_name: string;
  input_cost_usd: number;
  cached_input_cost_usd: number;
  output_cost_usd: number;
  total_cost_usd: number;
  usage: TokenUsage;
  cost_sparkline: number[];
};

type QuotaMode = "target" | "cap";

type QuotaSnapshot = {
  mode: QuotaMode;
  amount_usd: number;
  progress_ratio: number;
  primary_label: string;
  status_label: string;
  is_error_state: boolean;
};

type SnapshotWarning = {
  kind: string;
  message: string;
};

type HeatmapDay = {
  date: string;
  total_cost_usd: number;
};

type UsageHeatmap = {
  provider_id: string;
  today: string;
  days: HeatmapDay[];
};

type AppSnapshot = {
  provider_id: string;
  enabled_provider_ids: string[];
  date: string;
  title: string;
  tooltip: string;
  total_cost_usd: number;
  total_cost_sparkline: number[];
  totals: TokenUsage;
  model_costs: CostBreakdown[];
  pricing_updated_at: string | null;
  used_stale_pricing: boolean;
  last_refreshed_at: string;
  quota: QuotaSnapshot | null;
  dashboard_always_on_top: boolean;
  warning: SnapshotWarning | null;
  error_message: string | null;
};

type WindowView = "dashboard" | "settings";

type NavigateEvent = {
  view: WindowView;
  reloadSettings?: boolean;
};

let shellEl: HTMLElement | null;
let topActionsEl: HTMLElement | null;
let dashboardViewEl: HTMLElement | null;
let settingsViewEl: HTMLElement | null;
let settingsBackButtonEl: HTMLButtonElement | null;
let summaryEl: HTMLElement | null;
let summaryLabelEl: HTMLElement | null;
let summaryTrendEl: HTMLElement | null;
let summaryTriggerEl: HTMLElement | null;
let heroMainEl: HTMLElement | null;
let heatmapPanelEl: HTMLElement | null;
let totalsEl: HTMLElement | null;
let quotaRowEl: HTMLElement | null;
let modelsEl: HTMLElement | null;
let providerSwitcherEl: HTMLElement | null;
let heatmapButtonEl: HTMLButtonElement | null;
let settingsButtonEl: HTMLButtonElement | null;
let pinButtonEl: HTMLButtonElement | null;
let shareButtonEl: HTMLButtonElement | null;
let toastEl: HTMLElement | null;
let toastTimer: number | undefined;
let currentProviderId = "codex";
let providerSwitchInFlight = false;
let liveSnapshot: AppSnapshot | null = null;
let renderedSnapshot: AppSnapshot | null = null;
let heatmapData: UsageHeatmap | null = null;
let heatmapVisible = false;
let heatmapLoading = false;
let heatmapWarmupRequestedForProvider: string | null = null;
let currentView: WindowView = "dashboard";

const PROVIDERS = [
  { id: "codex", label: "Codex" },
  { id: "claude", label: "Claude Code" },
  { id: "kimi", label: "Kimi Code" },
] as const;

function usd(value: number) {
  return `$${value.toFixed(2)}`;
}

function providerLabel(providerId: string) {
  return PROVIDERS.find((provider) => provider.id === providerId)?.label ?? "Codex";
}

function formatShortDate(value: string) {
  const parsed = new Date(`${value}T00:00:00`);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" }).format(parsed);
}

function formatTokens(value: number) {
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(1)}M`;
  }
  if (value >= 1_000) {
    return `${(value / 1_000).toFixed(1)}K`;
  }
  return value.toString();
}

function iconMarkup(kind: "input" | "cached" | "output") {
  if (kind === "input") {
    return `
      <svg viewBox="0 0 16 16" fill="none" aria-hidden="true">
        <path d="M8 13V3.2M8 3.2 4.5 6.7M8 3.2l3.5 3.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
      </svg>
    `;
  }

  if (kind === "cached") {
    return `
      <svg viewBox="0 0 16 16" fill="none" aria-hidden="true">
        <path d="M9.2 1.8 4.6 7.5h2.9l-.8 6.7 4.7-5.8H8.5l.7-6.6Z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/>
      </svg>
    `;
  }

  return `
    <svg viewBox="0 0 16 16" fill="none" aria-hidden="true">
      <path d="M8 3v9.8M8 12.8l-3.5-3.5M8 12.8l3.5-3.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
    </svg>
  `;
}

function buildPath(points: number[], width: number, height: number, startIndex: number, endIndex: number) {
  if (points.length === 0 || endIndex < startIndex) {
    return "";
  }

  const baselineY = height - 3;
  const maxValue = Math.max(...points, 0);
  const stepX = points.length > 1 ? width / (points.length - 1) : width;

  return points
    .slice(startIndex, endIndex + 1)
    .map((value, offset) => {
      const index = startIndex + offset;
      const x = Number((index * stepX).toFixed(2));
      const y = maxValue > 0 ? Number((baselineY - (value / maxValue) * (height - 6)).toFixed(2)) : baselineY;
      return `${offset === 0 ? "M" : "L"} ${x} ${y}`;
    })
    .join(" ");
}

function currentHalfHourBucket(timestamp: string) {
  const parsed = new Date(timestamp);
  if (Number.isNaN(parsed.getTime())) {
    return 47;
  }

  const bucket = parsed.getHours() * 2 + (parsed.getMinutes() >= 30 ? 1 : 0);
  return Math.max(0, Math.min(47, bucket));
}

function currentBucketForSnapshot(snapshot: AppSnapshot) {
  const today = new Date().toLocaleDateString("en-CA");
  if (snapshot.date !== today) {
    return 47;
  }

  return currentHalfHourBucket(snapshot.last_refreshed_at);
}

function sparklineMarkup(points: number[], currentBucket: number) {
  const width = 136;
  const height = 28;
  const baselineColor = "rgba(244, 239, 229, 0.05)";
  const lineColor = "rgba(243, 169, 75, 0.52)";
  const values = points.length ? points : [0];
  const baselineY = height - 3;
  const clampedBucket = Math.max(0, Math.min(values.length - 1, currentBucket));
  const stepX = values.length > 1 ? width / (values.length - 1) : width;
  const currentX = Number((clampedBucket * stepX).toFixed(2));
  const solidPath = buildPath(values, width, height, 0, clampedBucket);
  const dashedPath =
    clampedBucket < values.length - 1
      ? buildPath(values, width, height, clampedBucket, values.length - 1)
      : "";

  return `
    <svg class="model-sparkline" viewBox="0 0 ${width} ${height}" preserveAspectRatio="none" aria-hidden="true">
      <path d="M 0 ${baselineY} L ${currentX} ${baselineY}" fill="none" stroke="${baselineColor}" stroke-width="1"></path>
      ${
        clampedBucket < values.length - 1
          ? `<path d="M ${currentX} ${baselineY} L ${width} ${baselineY}" fill="none" stroke="${baselineColor}" stroke-width="1" stroke-linecap="round" stroke-dasharray="2.5 3.5"></path>`
          : ""
      }
      <path d="${solidPath}" fill="none" stroke="${lineColor}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"></path>
      ${
        dashedPath
          ? `<path d="${dashedPath}" fill="none" stroke="${lineColor}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" stroke-dasharray="2.5 3.5"></path>`
          : ""
      }
    </svg>
  `;
}

function billableInputTokens(usage: TokenUsage) {
  return Math.max(0, usage.input_tokens - usage.cached_input_tokens + usage.cache_creation_input_tokens);
}

function totalOutputTokens(usage: TokenUsage) {
  return usage.output_tokens + usage.reasoning_output_tokens;
}

function normalizeLucide(svg: string) {
  return svg.replace("<svg ", '<svg fill="none" aria-hidden="true" ');
}

function pinIconMarkup(pinned: boolean) {
  return normalizeLucide(pinned ? pinIcon : pinOffIcon);
}

function screenshotIconMarkup() {
  return normalizeLucide(shareIcon);
}

function settingsIconMarkup() {
  return normalizeLucide(settingsIcon);
}

function backIconMarkup() {
  return normalizeLucide(arrowLeftIcon);
}

function heatmapIconMarkup() {
  return normalizeLucide(calendarIcon);
}

function loadingIconMarkup() {
  return normalizeLucide(loaderIcon);
}

function emptyStateMarkup(snapshot: AppSnapshot) {
  if (snapshot.provider_id === "claude" && snapshot.warning) {
    return `<div class="empty is-warning">${snapshot.warning.message}</div>`;
  }

  if (isInitialLoadingSnapshot(snapshot)) {
    return `<div class="empty">Analyzing local usage…</div>`;
  }

  return `<div class="empty">No ${providerLabel(snapshot.provider_id)} usage found for today.</div>`;
}

function isInitialLoadingSnapshot(snapshot: AppSnapshot) {
  return (
    snapshot.title === "--"
    && !snapshot.error_message
    && snapshot.model_costs.length === 0
    && snapshot.total_cost_usd === 0
  );
}

function buildHeatmapGrid(days: HeatmapDay[]) {
  if (!days.length) {
    return { monthLabels: "", weeksMarkup: "", weekCount: 0 };
  }

  const byDate = new Map(days.map((day) => [day.date, day]));
  const first = new Date(`${days[0].date}T00:00:00`);
  const last = new Date(`${days[days.length - 1].date}T00:00:00`);
  const start = new Date(first);
  start.setDate(start.getDate() - start.getDay());
  const end = new Date(last);
  end.setDate(end.getDate() + (6 - end.getDay()));

  const allDays: Array<HeatmapDay | null> = [];
  for (let cursor = new Date(start); cursor <= end; cursor.setDate(cursor.getDate() + 1)) {
    const key = cursor.toISOString().slice(0, 10);
    allDays.push(byDate.get(key) ?? null);
  }

  const weeks: Array<Array<HeatmapDay | null>> = [];
  for (let index = 0; index < allDays.length; index += 7) {
    weeks.push(allDays.slice(index, index + 7));
  }

  const maxCost = Math.max(...days.map((day) => day.total_cost_usd), 0);
  const levelFor = (value: number) => {
    if (value <= 0 || maxCost <= 0) return 0;
    const ratio = value / maxCost;
    if (ratio < 0.25) return 1;
    if (ratio < 0.5) return 2;
    if (ratio < 0.75) return 3;
    return 4;
  };

  const monthLabels = weeks
    .map((week, index) => {
      const firstRealDay = week.find(Boolean);
      if (!firstRealDay) {
        return `<span></span>`;
      }
      const date = new Date(`${firstRealDay.date}T00:00:00`);
      const showLabel = index === 0 || date.getDate() <= 7;
      return `<span>${showLabel ? date.toLocaleString(undefined, { month: "short" }) : ""}</span>`;
    })
    .join("");

  const weeksMarkup = weeks
    .map(
      (week) => `
        <div class="heatmap-week">
          ${week
            .map((day) => {
              if (!day) {
                return `<span class="heatmap-day is-empty" aria-hidden="true"></span>`;
              }

              const isSelected = renderedSnapshot?.date === day.date;
              return `
                <button
                  class="heatmap-day level-${levelFor(day.total_cost_usd)}${isSelected ? " is-selected" : ""}"
                  type="button"
                  data-heatmap-date="${day.date}"
                  title="${day.date}: ${usd(day.total_cost_usd)}"
                ></button>
              `;
            })
            .join("")}
        </div>
      `,
    )
    .join("");

  return { monthLabels, weeksMarkup, weekCount: weeks.length };
}

async function loadHeatmap(force = false) {
  if (!renderedSnapshot) {
    return;
  }

  if (!force && heatmapData?.provider_id === renderedSnapshot.provider_id) {
    return;
  }

  heatmapLoading = true;
  renderHeatmap();
  try {
    heatmapData = await invoke<UsageHeatmap>("get_usage_heatmap", {
      providerId: renderedSnapshot.provider_id,
      weeks: 26,
    });
  } catch (error) {
    console.error(error);
    showToast("Load dates failed", "error");
  } finally {
    heatmapLoading = false;
    renderHeatmap();
  }
}

function requestHeatmapWarmup() {
  if (!renderedSnapshot) {
    return;
  }

  const providerId = renderedSnapshot.provider_id;
  if (heatmapWarmupRequestedForProvider === providerId) {
    return;
  }

  heatmapWarmupRequestedForProvider = providerId;
  renderHeatmap();
  void invoke("warm_usage_history", {
    providerId,
    weeks: 26,
  }).catch((error) => {
    console.error(error);
    heatmapWarmupRequestedForProvider = null;
    renderHeatmap();
  });
}

function isHeatmapBusyForCurrentProvider() {
  return Boolean(
    renderedSnapshot
    && (heatmapLoading || heatmapWarmupRequestedForProvider === renderedSnapshot.provider_id),
  );
}

function renderHeatmap() {
  if (!heatmapPanelEl || !summaryTriggerEl || !heroMainEl || !heatmapButtonEl || !renderedSnapshot) {
    return;
  }

  summaryTriggerEl.setAttribute("aria-expanded", String(heatmapVisible));
  heroMainEl.classList.toggle("is-heatmap-open", heatmapVisible);
  heatmapButtonEl.setAttribute("aria-pressed", String(heatmapVisible));
  heatmapButtonEl.classList.toggle("is-active", heatmapVisible);
  heatmapButtonEl.classList.toggle("is-loading", isHeatmapBusyForCurrentProvider());
  heatmapButtonEl.innerHTML = isHeatmapBusyForCurrentProvider() ? loadingIconMarkup() : heatmapIconMarkup();
  heatmapPanelEl.hidden = !heatmapVisible;
  if (!heatmapVisible) {
    return;
  }

  if (heatmapLoading) {
    heatmapPanelEl.innerHTML = `<div class="heatmap-empty">Loading dates…</div>`;
    return;
  }

  if (!heatmapData || !heatmapData.days.length) {
    heatmapPanelEl.innerHTML = `<div class="heatmap-empty">No recent history.</div>`;
    return;
  }

  const { monthLabels, weeksMarkup, weekCount } = buildHeatmapGrid(heatmapData.days);
  const isToday = renderedSnapshot.date === heatmapData.today;

  heatmapPanelEl.innerHTML = `
    <div class="heatmap-head">
      <div class="heatmap-caption">Recent dates</div>
      <button class="heatmap-today" type="button" ${isToday ? "disabled" : ""}>Today</button>
    </div>
    <div class="heatmap-calendar">
      <div class="heatmap-months" style="grid-template-columns: repeat(${weekCount}, var(--heatmap-cell-size));">${monthLabels}</div>
      <div class="heatmap-grid">${weeksMarkup}</div>
    </div>
  `;
}

function showToast(message: string, kind: "success" | "error" = "success") {
  if (!toastEl) {
    return;
  }

  if (toastTimer) {
    window.clearTimeout(toastTimer);
  }

  toastEl.innerHTML = `<div class="toast-message${kind === "error" ? " error" : ""}">${message}</div>`;
  toastEl.classList.add("visible");
  toastTimer = window.setTimeout(() => {
    toastEl?.classList.remove("visible");
  }, 1800);
}

async function copyDashboardSnapshot() {
  if (!shareButtonEl) {
    return;
  }

  const target = document.querySelector<HTMLElement>("#capture-target");
  if (!target) {
    showToast("Copy failed", "error");
    return;
  }

  shareButtonEl.disabled = true;

  try {
    const rect = target.getBoundingClientRect();
    const exportPadding = 28;
    const pngDataUrl = await toPng(target, {
      cacheBust: true,
      pixelRatio: Math.min(window.devicePixelRatio || 1, 2),
      backgroundColor: "#171a1f",
      width: Math.ceil(rect.width) + exportPadding * 2,
      height: Math.ceil(rect.height) + exportPadding * 2,
      filter: (node: Node) => !(node instanceof HTMLElement && node.id === "status"),
      style: {
        boxSizing: "border-box",
        padding: `${exportPadding}px`,
        background:
          "radial-gradient(circle at top left, rgba(242, 146, 29, 0.18), transparent 22rem), linear-gradient(180deg, #121417 0%, #171a1f 100%)",
      },
    });
    const base64 = pngDataUrl.split(",", 2)[1];
    if (!base64) {
      throw new Error("Invalid image output");
    }

    await invoke("copy_dashboard_image_to_clipboard", { pngBase64: base64 });
    showToast("Copied to clipboard");
  } catch (error) {
    console.error(error);
    showToast("Copy failed", "error");
  } finally {
    shareButtonEl.disabled = false;
  }
}

function render(snapshot: AppSnapshot) {
  renderedSnapshot = snapshot;
  currentProviderId = snapshot.provider_id;
  const currentBucket = currentBucketForSnapshot(snapshot);
  const isToday =
    (heatmapData && snapshot.date === heatmapData.today)
    || (liveSnapshot
      && snapshot.provider_id === liveSnapshot.provider_id
      && snapshot.date === liveSnapshot.date);

  if (providerSwitcherEl) {
    const enabledProviders = PROVIDERS.filter((provider) =>
      snapshot.enabled_provider_ids.includes(provider.id),
    );
    providerSwitcherEl.hidden = enabledProviders.length <= 1;
    providerSwitcherEl.innerHTML = enabledProviders.map(
      (provider) => `
        <button
          class="provider-chip${provider.id === snapshot.provider_id ? " is-active" : ""}"
          type="button"
          role="tab"
          aria-selected="${provider.id === snapshot.provider_id}"
          data-provider-id="${provider.id}"
          ${providerSwitchInFlight ? "disabled" : ""}
        >
          ${provider.label}
        </button>
      `,
    ).join("");
  }

  if (summaryEl) {
    summaryEl.textContent = usd(snapshot.total_cost_usd);
  }

  if (summaryLabelEl) {
    summaryLabelEl.textContent = isToday ? "Today" : formatShortDate(snapshot.date);
  }

  if (pinButtonEl) {
    pinButtonEl.innerHTML = pinIconMarkup(snapshot.dashboard_always_on_top);
    pinButtonEl.classList.toggle("is-active", snapshot.dashboard_always_on_top);
    pinButtonEl.setAttribute(
      "aria-label",
      snapshot.dashboard_always_on_top ? "Disable always on top" : "Enable always on top",
    );
    pinButtonEl.title = snapshot.dashboard_always_on_top ? "Always on top is on" : "Keep window on top";
  }

  if (summaryTrendEl) {
    const showSummaryTrend = isToday;
    summaryTrendEl.hidden = !showSummaryTrend;
    summaryTrendEl.innerHTML = showSummaryTrend
      ? sparklineMarkup(snapshot.total_cost_sparkline, currentBucket)
      : "";
  }

  const summaryRowEl = summaryEl?.closest(".summary-row");
  if (summaryRowEl) {
    summaryRowEl.classList.toggle("is-history", !isToday);
  }

  if (quotaRowEl) {
    if (!snapshot.quota) {
      quotaRowEl.hidden = true;
      quotaRowEl.innerHTML = "";
    } else {
      const progress = `${Math.max(0, Math.min(snapshot.quota.progress_ratio, 1)) * 100}%`;
      quotaRowEl.hidden = false;
      quotaRowEl.innerHTML = `
        <div class="quota-copy">
          <strong>${snapshot.quota.primary_label}</strong>
          <span>${snapshot.quota.status_label}</span>
        </div>
        <div class="quota-progress${snapshot.quota.is_error_state ? " is-error" : ""}">
          <div class="quota-progress-fill" style="width: ${snapshot.quota.is_error_state ? "0%" : progress};"></div>
        </div>
      `;
    }
  }

  if (totalsEl) {
    const billableInput = billableInputTokens(snapshot.totals);
    totalsEl.innerHTML = `
      <div class="metric-item metric-item-with-icon">
        <div class="metric-label-row">
          <span class="metric-icon metric-input">${iconMarkup("input")}</span>
          <span>Input</span>
        </div>
        <strong>${formatTokens(billableInput)}</strong>
      </div>
      <div class="metric-item metric-item-with-icon">
        <div class="metric-label-row">
          <span class="metric-icon metric-cached">${iconMarkup("cached")}</span>
          <span>Cached</span>
        </div>
        <strong>${formatTokens(snapshot.totals.cached_input_tokens)}</strong>
      </div>
      <div class="metric-item metric-item-with-icon">
        <div class="metric-label-row">
          <span class="metric-icon metric-output">${iconMarkup("output")}</span>
          <span>Output</span>
        </div>
        <strong>${formatTokens(totalOutputTokens(snapshot.totals))}</strong>
      </div>
    `;
  }

  if (modelsEl) {
    if (!snapshot.model_costs.length) {
      modelsEl.innerHTML = emptyStateMarkup(snapshot);
      return;
    }

    modelsEl.innerHTML = snapshot.model_costs
      .map(
        (item) => `
          <article class="model-card">
            <div class="model-row${isToday ? "" : " is-history"}">
              <h3>${item.model_name}</h3>
              ${isToday ? `<div class="model-trend">${sparklineMarkup(item.cost_sparkline, currentBucket)}</div>` : ""}
              <strong>${usd(item.total_cost_usd)}</strong>
            </div>
            <div class="model-metrics">
              <span class="metric-pill metric-input">
                <span class="metric-icon">${iconMarkup("input")}</span>
                <span>${formatTokens(billableInputTokens(item.usage))}</span>
              </span>
              <span class="metric-pill metric-cached">
                <span class="metric-icon">${iconMarkup("cached")}</span>
                <span>${formatTokens(item.usage.cached_input_tokens)}</span>
              </span>
              <span class="metric-pill metric-output">
                <span class="metric-icon">${iconMarkup("output")}</span>
                <span>${formatTokens(totalOutputTokens(item.usage))}</span>
              </span>
            </div>
          </article>
        `,
      )
      .join("");
  }

  renderHeatmap();
}

async function setView(view: WindowView, options: { reloadSettings?: boolean } = {}) {
  currentView = view;
  document.body.dataset.view = view;
  shellEl?.setAttribute("data-view", view);

  if (dashboardViewEl) {
    dashboardViewEl.hidden = view !== "dashboard";
  }

  if (settingsViewEl) {
    settingsViewEl.hidden = view !== "settings";
  }

  if (settingsButtonEl) {
    settingsButtonEl.classList.toggle("is-active", view === "settings");
    settingsButtonEl.setAttribute(
      "aria-label",
      view === "settings" ? "Settings view is open" : "Open settings",
    );
  }

  if (topActionsEl) {
    topActionsEl.hidden = view === "settings";
  }

  if (view === "settings") {
    heatmapVisible = false;
    renderHeatmap();
    if (options.reloadSettings) {
      await loadSettingsView();
    }
    return;
  }

  if (liveSnapshot) {
    render(liveSnapshot);
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  shellEl = document.querySelector(".shell");
  topActionsEl = document.querySelector("#top-actions");
  dashboardViewEl = document.querySelector("#dashboard-view");
  settingsViewEl = document.querySelector("#settings-view");
  settingsBackButtonEl = document.querySelector("#settings-back-button");
  summaryEl = document.querySelector("#summary");
  summaryLabelEl = document.querySelector("#summary-label");
  summaryTrendEl = document.querySelector("#summary-trend");
  summaryTriggerEl = document.querySelector("#summary-trigger");
  heroMainEl = document.querySelector("#hero-main");
  heatmapPanelEl = document.querySelector("#heatmap-panel");
  totalsEl = document.querySelector("#totals");
  quotaRowEl = document.querySelector("#quota-row");
  modelsEl = document.querySelector("#models");
  providerSwitcherEl = document.querySelector("#provider-switcher");
  heatmapButtonEl = document.querySelector("#heatmap-button");
  settingsButtonEl = document.querySelector("#settings-button");
  pinButtonEl = document.querySelector("#pin-button");
  shareButtonEl = document.querySelector("#share-button");
  toastEl = document.querySelector("#toast");

  if (settingsButtonEl) {
    settingsButtonEl.innerHTML = settingsIconMarkup();
  }

  if (settingsBackButtonEl) {
    settingsBackButtonEl.innerHTML = `${backIconMarkup()}<span>Back</span>`;
  }

  if (shareButtonEl) {
    shareButtonEl.innerHTML = screenshotIconMarkup();
  }

  if (heatmapButtonEl) {
    heatmapButtonEl.innerHTML = heatmapIconMarkup();
  }

  initializeSettingsView();

  settingsButtonEl?.addEventListener("click", async () => {
    await loadSettingsView();
    await setView("settings", { reloadSettings: false });
  });

  settingsBackButtonEl?.addEventListener("click", () => {
    void setView("dashboard");
  });

  pinButtonEl?.addEventListener("click", async () => {
    pinButtonEl!.disabled = true;
    try {
      const snapshot = await invoke<AppSnapshot>("toggle_dashboard_always_on_top");
      render(snapshot);
    } catch (error) {
      console.error(error);
      showToast("Toggle failed", "error");
    } finally {
      pinButtonEl!.disabled = false;
    }
  });

  shareButtonEl?.addEventListener("click", () => {
    void copyDashboardSnapshot();
  });

  heatmapButtonEl?.addEventListener("click", async () => {
    heatmapVisible = !heatmapVisible;
    renderHeatmap();
    if (heatmapVisible) {
      await loadHeatmap(true);
      requestHeatmapWarmup();
    }
  });

  heatmapPanelEl?.addEventListener("click", async (event) => {
    const target = event.target as HTMLElement;
    const dateButton = target.closest<HTMLElement>("[data-heatmap-date]");
    if (dateButton?.dataset.heatmapDate && renderedSnapshot) {
      const snapshot = await invoke<AppSnapshot>("get_snapshot_for_date", {
        providerId: renderedSnapshot.provider_id,
        date: dateButton.dataset.heatmapDate,
      });
      render(snapshot);
      return;
    }

    if (target.closest(".heatmap-today") && liveSnapshot) {
      render(liveSnapshot);
    }
  });

  providerSwitcherEl?.addEventListener("click", async (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-provider-id]");
    const providerId = button?.dataset.providerId;
    if (!providerId || providerId === currentProviderId || providerSwitchInFlight) {
      return;
    }

    providerSwitchInFlight = true;
    if (liveSnapshot) {
      render(liveSnapshot);
    }

    try {
      const snapshot = await invoke<AppSnapshot>("set_current_provider", { providerId });
      liveSnapshot = snapshot;
      heatmapData = null;
      heatmapWarmupRequestedForProvider = null;
      render(snapshot);
      if (heatmapVisible) {
        await loadHeatmap(true);
        requestHeatmapWarmup();
      }
    } catch (error) {
      console.error(error);
      showToast("Switch failed", "error");
    } finally {
      providerSwitchInFlight = false;
      if (liveSnapshot && renderedSnapshot?.date === liveSnapshot.date && renderedSnapshot.provider_id === liveSnapshot.provider_id) {
        render(liveSnapshot);
      } else if (currentView === "dashboard") {
        renderHeatmap();
      }
    }
  });

  const snapshot = await invoke<AppSnapshot>("get_snapshot");
  liveSnapshot = snapshot;
  heatmapData = null;
  heatmapWarmupRequestedForProvider = null;
  render(snapshot);
  await setView("dashboard");

  await listen<AppSnapshot>("snapshot-updated", (event) => {
    liveSnapshot = event.payload;
    if (
      currentView === "dashboard"
      && (
      !renderedSnapshot
      || (renderedSnapshot.provider_id === event.payload.provider_id
        && renderedSnapshot.date === event.payload.date)
      )
    ) {
      render(event.payload);
    } else if (
      currentView === "dashboard"
      && heatmapVisible
      && heatmapData?.provider_id === event.payload.provider_id
    ) {
      void loadHeatmap(true);
    }
  });

  await listen<NavigateEvent>("navigate", (event) => {
    void (async () => {
      if (event.payload.view === "settings" && event.payload.reloadSettings) {
        await loadSettingsView();
      }
      await setView(event.payload.view, {
        reloadSettings: false,
      });
    })();
  });

  await listen<{ providerId: string }>("usage-history-warmed", (event) => {
    if (heatmapWarmupRequestedForProvider === event.payload.providerId) {
      heatmapWarmupRequestedForProvider = null;
    }

    if (!renderedSnapshot || renderedSnapshot.provider_id !== event.payload.providerId) {
      return;
    }

    renderHeatmap();
    if (heatmapVisible) {
      void loadHeatmap(true);
    }
  });
});
