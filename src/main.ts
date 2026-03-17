import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type TokenUsage = {
  input_tokens: number;
  cached_input_tokens: number;
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
};

type AppSnapshot = {
  provider_id: string;
  date: string;
  title: string;
  tooltip: string;
  total_cost_usd: number;
  totals: TokenUsage;
  model_costs: CostBreakdown[];
  pricing_updated_at: string | null;
  used_stale_pricing: boolean;
  last_refreshed_at: string;
  error_message: string | null;
};

let summaryEl: HTMLElement | null;
let totalsEl: HTMLElement | null;
let modelsEl: HTMLElement | null;
let statusEl: HTMLElement | null;
let idleStatusText = "Waiting for data...";

function usd(value: number) {
  return `$${value.toFixed(4)}`;
}

function formatTokens(value: number) {
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1)}M`;
  }
  if (value >= 1_000) {
    return `${(value / 1_000).toFixed(value >= 10_000 ? 0 : 1)}K`;
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

function formatRelativeTime(timestamp: string) {
  const target = new Date(timestamp);
  if (Number.isNaN(target.getTime())) {
    return timestamp;
  }

  const diffMs = target.getTime() - Date.now();
  const diffMinutes = Math.round(diffMs / 60_000);
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

  if (Math.abs(diffMinutes) < 1) {
    return "just now";
  }

  if (Math.abs(diffMinutes) < 60) {
    return rtf.format(diffMinutes, "minute");
  }

  const diffHours = Math.round(diffMinutes / 60);
  if (Math.abs(diffHours) < 24) {
    return rtf.format(diffHours, "hour");
  }

  const diffDays = Math.round(diffHours / 24);
  return rtf.format(diffDays, "day");
}

function billableInputTokens(usage: TokenUsage) {
  return Math.max(0, usage.input_tokens - usage.cached_input_tokens);
}

function totalOutputTokens(usage: TokenUsage) {
  return usage.output_tokens + usage.reasoning_output_tokens;
}

function render(snapshot: AppSnapshot) {
  idleStatusText = snapshot.error_message
    ? snapshot.error_message
    : `Updated ${formatRelativeTime(snapshot.last_refreshed_at)}`;

  if (summaryEl) {
    summaryEl.textContent = usd(snapshot.total_cost_usd);
  }

  if (statusEl) {
    statusEl.textContent = idleStatusText;
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
      modelsEl.innerHTML = `<div class="empty">No usage found for today.</div>`;
      return;
    }

    modelsEl.innerHTML = snapshot.model_costs
      .map(
        (item) => `
          <article class="model-card">
            <div class="model-row">
              <h3>${item.model_name}</h3>
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
}

window.addEventListener("DOMContentLoaded", async () => {
  summaryEl = document.querySelector("#summary");
  totalsEl = document.querySelector("#totals");
  modelsEl = document.querySelector("#models");
  statusEl = document.querySelector("#status");

  const snapshot = await invoke<AppSnapshot>("get_snapshot");
  render(snapshot);

  await listen<AppSnapshot>("snapshot-updated", (event) => {
    render(event.payload);
  });
});
