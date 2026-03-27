mod domain;
mod pricing;
mod providers;
mod service;
mod settings;
mod snapshot_store;

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use arboard::{Clipboard, ImageData};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Local};
use domain::{
    AppSnapshot, DashboardSettings, ProviderQuotaSettings, ProviderSettingsSummary, UsageHeatmap,
};
use service::{
    billable_input_tokens, build_error_snapshot, format_token_count, provider_display_name,
    total_output_tokens, UsageAppService,
};
use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::WindowEvent;
use tauri::{AppHandle, Emitter, Manager};

const TRAY_ID: &str = "usage-tray";
const MENU_STATUS: &str = "status";
const MENU_TOKENS: &str = "tokens";
const MENU_UPDATED: &str = "updated";
const MENU_REFRESH: &str = "refresh";
const MENU_SHOW: &str = "show";
const MENU_SETTINGS: &str = "settings";
const MENU_QUIT: &str = "quit";

struct AppState {
    service: Arc<UsageAppService>,
    snapshot: Arc<Mutex<AppSnapshot>>,
    status_item: Arc<Mutex<Option<MenuItem<tauri::Wry>>>>,
    tokens_item: Arc<Mutex<Option<MenuItem<tauri::Wry>>>>,
    updated_item: Arc<Mutex<Option<MenuItem<tauri::Wry>>>>,
    warming_history_providers: Arc<Mutex<HashSet<String>>>,
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum WindowView {
    Dashboard,
    Settings,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NavigateEvent {
    view: WindowView,
    #[serde(skip_serializing_if = "Option::is_none")]
    reload_settings: Option<bool>,
}

impl AppState {
    fn new() -> Self {
        let service = UsageAppService::new().expect("failed to initialize service");
        let dashboard_settings = service.load_dashboard_settings().unwrap_or_default();
        Self {
            service: Arc::new(service),
            snapshot: Arc::new(Mutex::new(build_loading_snapshot(&dashboard_settings))),
            status_item: Arc::new(Mutex::new(None)),
            tokens_item: Arc::new(Mutex::new(None)),
            updated_item: Arc::new(Mutex::new(None)),
            warming_history_providers: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

#[tauri::command]
fn get_snapshot(state: tauri::State<'_, AppState>) -> AppSnapshot {
    state.snapshot.lock().unwrap().clone()
}

#[tauri::command]
fn get_dashboard_settings(state: tauri::State<'_, AppState>) -> Result<DashboardSettings, String> {
    state
        .service
        .load_dashboard_settings()
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
fn get_provider_quota_settings(
    state: tauri::State<'_, AppState>,
) -> Result<ProviderQuotaSettings, String> {
    state
        .service
        .load_provider_quota_settings()
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
fn get_provider_settings_summaries(
    state: tauri::State<'_, AppState>,
) -> Vec<ProviderSettingsSummary> {
    state.service.load_provider_settings_summaries()
}

#[tauri::command]
fn get_snapshot_for_date(
    state: tauri::State<'_, AppState>,
    provider_id: String,
    date: String,
) -> Result<AppSnapshot, String> {
    let parsed_date = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|error| format!("invalid date: {error}"))?;

    state
        .service
        .snapshot_for_date(&provider_id, parsed_date)
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
fn get_usage_heatmap(
    state: tauri::State<'_, AppState>,
    provider_id: String,
    weeks: Option<usize>,
) -> Result<UsageHeatmap, String> {
    state
        .service
        .load_usage_heatmap(&provider_id, weeks.unwrap_or(26))
        .map_err(|error| format!("{error:#}"))
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryWarmEvent {
    provider_id: String,
}

#[tauri::command]
fn warm_usage_history(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    provider_id: String,
    weeks: Option<usize>,
) -> Result<(), String> {
    let weeks = weeks.unwrap_or(26);
    {
        let mut inflight = state.warming_history_providers.lock().unwrap();
        if !inflight.insert(provider_id.clone()) {
            return Ok(());
        }
    }

    let service = state.service.clone();
    let inflight = state.warming_history_providers.clone();
    thread::spawn(move || {
        let result = service.warm_usage_history(&provider_id, weeks);
        inflight.lock().unwrap().remove(&provider_id);

        if let Err(error) = result {
            eprintln!(
                "failed to warm usage history for {}: {error:#}",
                provider_id
            );
            return;
        }

        let _ = app.emit(
            "usage-history-warmed",
            HistoryWarmEvent {
                provider_id: provider_id.clone(),
            },
        );
    });

    Ok(())
}

#[tauri::command]
fn save_provider_quota_settings(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    settings: ProviderQuotaSettings,
) -> Result<ProviderQuotaSettings, String> {
    let saved = state
        .service
        .save_provider_quota_settings(&settings)
        .map_err(|error| format!("{error:#}"))?;

    let snapshot = refresh_state(&app, &state, false);
    let _ = emit_snapshot(&app, &snapshot);

    Ok(saved)
}

#[tauri::command]
fn save_dashboard_settings(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    settings: DashboardSettings,
) -> Result<DashboardSettings, String> {
    let saved = state
        .service
        .save_dashboard_settings(&settings)
        .map_err(|error| format!("{error:#}"))?;

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_always_on_top(saved.always_on_top);
    }

    {
        let mut snapshot = state.snapshot.lock().unwrap();
        snapshot.provider_id = saved.current_provider.clone();
        snapshot.enabled_provider_ids = saved.enabled_providers.clone();
        snapshot.dashboard_always_on_top = saved.always_on_top;
    }

    let snapshot = refresh_state(&app, &state, false);
    let _ = emit_snapshot(&app, &snapshot);

    Ok(saved)
}

#[tauri::command]
fn toggle_dashboard_always_on_top(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let current = state
        .service
        .load_dashboard_settings()
        .map_err(|error| format!("{error:#}"))?;
    let updated = DashboardSettings {
        always_on_top: !current.always_on_top,
        current_provider: current.current_provider,
        enabled_providers: current.enabled_providers,
    };

    state
        .service
        .save_dashboard_settings(&updated)
        .map_err(|error| format!("{error:#}"))?;

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_always_on_top(updated.always_on_top);
    }

    {
        let mut snapshot = state.snapshot.lock().unwrap();
        snapshot.provider_id = updated.current_provider.clone();
        snapshot.enabled_provider_ids = updated.enabled_providers.clone();
        snapshot.dashboard_always_on_top = updated.always_on_top;
    }

    let snapshot = refresh_state(&app, &state, false);
    let _ = emit_snapshot(&app, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn set_current_provider(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    provider_id: String,
) -> Result<AppSnapshot, String> {
    if !matches!(provider_id.as_str(), "codex" | "claude" | "kimi") {
        return Err(format!("unsupported provider: {provider_id}"));
    }

    let current = state
        .service
        .load_dashboard_settings()
        .map_err(|error| format!("{error:#}"))?;
    if !current
        .enabled_providers
        .iter()
        .any(|enabled| enabled == &provider_id)
    {
        return Err(format!("provider is disabled: {provider_id}"));
    }
    let updated = DashboardSettings {
        always_on_top: current.always_on_top,
        current_provider: provider_id,
        enabled_providers: current.enabled_providers,
    };

    state
        .service
        .save_dashboard_settings(&updated)
        .map_err(|error| format!("{error:#}"))?;

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_always_on_top(updated.always_on_top);
    }

    let loading_snapshot = build_loading_snapshot(&updated);
    {
        let mut snapshot = state.snapshot.lock().unwrap();
        *snapshot = loading_snapshot.clone();
    }
    let _ = apply_snapshot_to_tray(
        &app,
        &loading_snapshot,
        &state.status_item,
        &state.tokens_item,
        &state.updated_item,
    );
    let _ = emit_snapshot(&app, &loading_snapshot);

    let service = state.service.clone();
    let snapshot_store = state.snapshot.clone();
    let status_item = state.status_item.clone();
    let tokens_item = state.tokens_item.clone();
    let updated_item = state.updated_item.clone();
    let current_provider_id = loading_snapshot.provider_id.clone();
    let enabled_provider_ids = loading_snapshot.enabled_provider_ids.clone();

    thread::spawn(move || {
        let snapshot = match service.refresh(false) {
            Ok(snapshot) => snapshot,
            Err(error) => build_loading_error_snapshot(
                &current_provider_id,
                enabled_provider_ids,
                format!("{error:#}"),
            ),
        };

        *snapshot_store.lock().unwrap() = snapshot.clone();
        let _ = apply_snapshot_to_tray(&app, &snapshot, &status_item, &tokens_item, &updated_item);
        let _ = emit_snapshot(&app, &snapshot);
    });

    Ok(loading_snapshot)
}

#[tauri::command]
fn refresh_snapshot(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    force_pricing_refresh: bool,
) -> Result<AppSnapshot, String> {
    let current_snapshot = state.snapshot.lock().unwrap().clone();
    let current_provider_id = current_snapshot.provider_id.clone();
    let enabled_provider_ids = current_snapshot.enabled_provider_ids.clone();
    let service = state.service.clone();
    let snapshot_store = state.snapshot.clone();
    let status_item = state.status_item.clone();
    let tokens_item = state.tokens_item.clone();
    let updated_item = state.updated_item.clone();

    thread::spawn(move || {
        let snapshot = match service.refresh(force_pricing_refresh) {
            Ok(snapshot) => snapshot,
            Err(error) => build_loading_error_snapshot(
                &current_provider_id,
                enabled_provider_ids.clone(),
                format!("{error:#}"),
            ),
        };

        *snapshot_store.lock().unwrap() = snapshot.clone();
        let _ = apply_snapshot_to_tray(&app, &snapshot, &status_item, &tokens_item, &updated_item);
        let _ = emit_snapshot(&app, &snapshot);
    });

    Ok(current_snapshot)
}

#[tauri::command]
fn copy_dashboard_image_to_clipboard(png_base64: String) -> Result<(), String> {
    copy_png_base64_to_clipboard(&png_base64).map_err(|error| format!("{error:#}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::new())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_dashboard(app);
        }))
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        })
        .setup(|app| {
            build_tray(app)?;
            if let Some(window) = app.get_webview_window("main") {
                if let Ok(settings) = app.state::<AppState>().service.load_dashboard_settings() {
                    let _ = window.set_always_on_top(settings.always_on_top);
                }
                let _ = window.hide();
            }
            spawn_initial_refresh(app.handle().clone());
            spawn_periodic_refresh(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_dashboard_settings,
            get_provider_quota_settings,
            get_provider_settings_summaries,
            get_snapshot_for_date,
            get_usage_heatmap,
            warm_usage_history,
            refresh_snapshot,
            save_dashboard_settings,
            save_provider_quota_settings,
            toggle_dashboard_always_on_top,
            set_current_provider,
            copy_dashboard_image_to_clipboard
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows,
                ..
            } = _event
            {
                if !has_visible_windows {
                    show_dashboard(_app);
                }
            }
        });
}

fn build_tray(app: &mut tauri::App<tauri::Wry>) -> tauri::Result<()> {
    let status_item = MenuItem::with_id(app, MENU_STATUS, "Today: --", false, None::<&str>)?;
    let tokens_item = MenuItem::with_id(app, MENU_TOKENS, "Tokens: --", false, None::<&str>)?;
    let updated_item = MenuItem::with_id(app, MENU_UPDATED, "Updated: --", false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let refresh_item = MenuItem::with_id(app, MENU_REFRESH, "Refresh", true, None::<&str>)?;
    let show_item = MenuItem::with_id(app, MENU_SHOW, "Open dashboard", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, MENU_SETTINGS, "Settings", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;

    let menu = MenuBuilder::new(app)
        .items(&[
            &status_item,
            &tokens_item,
            &updated_item,
            &separator,
            &refresh_item,
            &show_item,
            &settings_item,
            &quit_item,
        ])
        .build()?;

    {
        let state = app.state::<AppState>();
        *state.status_item.lock().unwrap() = Some(status_item);
        *state.tokens_item.lock().unwrap() = Some(tokens_item);
        *state.updated_item.lock().unwrap() = Some(updated_item);
    }

    let icon = logo_tray_icon()?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("Loading usage…")
        .title("--")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } = event
            {
                let app = tray.app_handle();
                show_dashboard(&app);
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_REFRESH => {
                let state = app.state::<AppState>();
                let snapshot = refresh_state(app, &state, false);
                let _ = emit_snapshot(app, &snapshot);
            }
            MENU_SHOW => {
                show_dashboard(app);
            }
            MENU_SETTINGS => {
                show_settings(app);
            }
            MENU_QUIT => app.exit(0),
            _ => {}
        })
        .build(app)?;

    let state = app.state::<AppState>();
    let snapshot = state.snapshot.lock().unwrap().clone();
    let _ = apply_snapshot_to_tray(
        app.handle(),
        &snapshot,
        &state.status_item,
        &state.tokens_item,
        &state.updated_item,
    );
    emit_snapshot(app.handle(), &snapshot)?;

    Ok(())
}

fn spawn_initial_refresh(app: AppHandle) {
    let state = app.state::<AppState>();
    let service = state.service.clone();
    let snapshot_store = state.snapshot.clone();
    let status_item = state.status_item.clone();
    let tokens_item = state.tokens_item.clone();
    let updated_item = state.updated_item.clone();

    thread::spawn(move || {
        let provider_id = snapshot_store.lock().unwrap().provider_id.clone();
        let enabled_provider_ids = snapshot_store.lock().unwrap().enabled_provider_ids.clone();
        let snapshot = match service.refresh(false) {
            Ok(snapshot) => snapshot,
            Err(error) => build_loading_error_snapshot(
                &provider_id,
                enabled_provider_ids,
                format!("{error:#}"),
            ),
        };

        *snapshot_store.lock().unwrap() = snapshot.clone();
        let _ = apply_snapshot_to_tray(&app, &snapshot, &status_item, &tokens_item, &updated_item);
        let _ = emit_snapshot(&app, &snapshot);
    });
}

fn spawn_periodic_refresh(app: AppHandle) {
    thread::spawn(move || {
        let mut hidden_ticks = 0u8;

        loop {
            thread::sleep(Duration::from_secs(60));

            let is_dashboard_visible = app
                .get_webview_window("main")
                .and_then(|window| window.is_visible().ok())
                .unwrap_or(false);

            if is_dashboard_visible {
                hidden_ticks = 0;
                let state = app.state::<AppState>();
                let snapshot = refresh_state(&app, &state, false);
                let _ = emit_snapshot(&app, &snapshot);
                continue;
            }

            hidden_ticks = hidden_ticks.saturating_add(1);
            if hidden_ticks < 10 {
                continue;
            }

            hidden_ticks = 0;
            let state = app.state::<AppState>();
            let snapshot = refresh_state(&app, &state, false);
            let _ = emit_snapshot(&app, &snapshot);
        }
    });
}

fn refresh_state(
    app: &AppHandle,
    state: &tauri::State<'_, AppState>,
    force_pricing_refresh: bool,
) -> AppSnapshot {
    let current_snapshot = state.snapshot.lock().unwrap().clone();
    let current_provider_id = current_snapshot.provider_id;
    let enabled_provider_ids = current_snapshot.enabled_provider_ids;
    let snapshot = match state.service.refresh(force_pricing_refresh) {
        Ok(snapshot) => snapshot,
        Err(error) => build_loading_error_snapshot(
            &current_provider_id,
            enabled_provider_ids,
            format!("{error:#}"),
        ),
    };

    *state.snapshot.lock().unwrap() = snapshot.clone();
    let _ = apply_snapshot_to_tray(
        app,
        &snapshot,
        &state.status_item,
        &state.tokens_item,
        &state.updated_item,
    );
    snapshot
}

fn apply_snapshot_to_tray(
    app: &AppHandle,
    snapshot: &AppSnapshot,
    status_item: &Arc<Mutex<Option<MenuItem<tauri::Wry>>>>,
    tokens_item: &Arc<Mutex<Option<MenuItem<tauri::Wry>>>>,
    updated_item: &Arc<Mutex<Option<MenuItem<tauri::Wry>>>>,
) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(Some(logo_tray_icon()?));
        let _ = tray.set_tooltip(Some(snapshot.tooltip.clone()));
        let _ = tray.set_title(Some(format_tray_title(snapshot)));
    }

    if let Some(item) = status_item.lock().unwrap().as_ref() {
        item.set_text(format!(
            "{}: ${:.2}",
            provider_display_name(&snapshot.provider_id),
            snapshot.total_cost_usd
        ))?;
    }

    if let Some(item) = tokens_item.lock().unwrap().as_ref() {
        item.set_text(format!(
            "Tokens: ↑ {}   ⚡ {}   ↓ {}",
            format_token_count(billable_input_tokens(&snapshot.totals)),
            format_token_count(snapshot.totals.cached_input_tokens),
            format_token_count(total_output_tokens(&snapshot.totals))
        ))?;
    }

    if let Some(item) = updated_item.lock().unwrap().as_ref() {
        let freshness = if snapshot.used_stale_pricing {
            "pricing stale"
        } else {
            "pricing fresh"
        };
        item.set_text(format!(
            "Updated: {} ({freshness})",
            format_relative_time(&snapshot.last_refreshed_at)
        ))?;
    }

    Ok(())
}

fn build_loading_snapshot(settings: &DashboardSettings) -> AppSnapshot {
    let now = Local::now();
    let provider_label = provider_display_name(&settings.current_provider);

    AppSnapshot {
        provider_id: settings.current_provider.clone(),
        enabled_provider_ids: settings.enabled_providers.clone(),
        date: now.date_naive().to_string(),
        title: "--".to_string(),
        tooltip: format!("Loading {provider_label} usage…"),
        total_cost_usd: 0.0,
        total_cost_sparkline: vec![0.0; 48],
        totals: Default::default(),
        model_costs: Vec::new(),
        pricing_updated_at: None,
        used_stale_pricing: false,
        last_refreshed_at: now.to_rfc3339(),
        quota: None,
        dashboard_always_on_top: settings.always_on_top,
        warning: None,
        error_message: None,
    }
}

fn build_loading_error_snapshot(
    provider_id: &str,
    enabled_provider_ids: Vec<String>,
    message: impl Into<String>,
) -> AppSnapshot {
    let mut snapshot = build_error_snapshot(message);
    snapshot.provider_id = provider_id.to_string();
    snapshot.enabled_provider_ids = enabled_provider_ids;
    snapshot.tooltip = format!(
        "{} error: {}",
        provider_display_name(provider_id),
        snapshot.tooltip
    );
    snapshot
}

fn format_tray_title(snapshot: &AppSnapshot) -> String {
    if snapshot.error_message.is_some() {
        "--".to_string()
    } else {
        format!("${:.2}", snapshot.total_cost_usd)
    }
}

fn emit_snapshot(app: &AppHandle, snapshot: &AppSnapshot) -> tauri::Result<()> {
    app.emit("snapshot-updated", snapshot)
}

fn emit_navigation(
    app: &AppHandle,
    view: WindowView,
    reload_settings: Option<bool>,
) -> tauri::Result<()> {
    app.emit(
        "navigate",
        NavigateEvent {
            view,
            reload_settings,
        },
    )
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn show_dashboard(app: &AppHandle) {
    show_main_window(app);
    let _ = emit_navigation(app, WindowView::Dashboard, None);
}

fn show_settings(app: &AppHandle) {
    show_main_window(app);
    let _ = emit_navigation(app, WindowView::Settings, Some(true));
}

fn logo_tray_icon() -> tauri::Result<Image<'static>> {
    Image::from_bytes(include_bytes!(
        "../../artifacts/branding/codex-cost-logo-mark.png"
    ))
}

fn format_relative_time(timestamp: &str) -> String {
    let Ok(parsed) = DateTime::parse_from_rfc3339(timestamp) else {
        return timestamp.to_string();
    };

    let now = Local::now();
    let then = parsed.with_timezone(&Local);
    let delta = now.signed_duration_since(then);
    let seconds = delta.num_seconds();

    if seconds < 60 {
        return "just now".to_string();
    }

    let minutes = delta.num_minutes();
    if minutes < 60 {
        return format!("{minutes}m ago");
    }

    let hours = delta.num_hours();
    if hours < 24 {
        return format!("{hours}h ago");
    }

    let days = delta.num_days();
    format!("{days}d ago")
}

fn copy_png_base64_to_clipboard(png_base64: &str) -> Result<()> {
    let png_bytes = STANDARD
        .decode(png_base64)
        .context("failed to decode screenshot payload")?;
    let image = image::load_from_memory_with_format(&png_bytes, image::ImageFormat::Png)
        .context("failed to decode screenshot image")?
        .to_rgba8();
    let (width, height) = image.dimensions();

    let mut clipboard = Clipboard::new().context("failed to access system clipboard")?;
    clipboard
        .set_image(ImageData {
            width: width as usize,
            height: height as usize,
            bytes: Cow::Owned(image.into_raw()),
        })
        .context("failed to copy image to clipboard")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
    use serde_json::json;

    use super::{
        build_loading_snapshot, copy_png_base64_to_clipboard, format_tray_title, NavigateEvent,
        WindowView,
    };
    use crate::domain::DashboardSettings;
    use crate::service::build_error_snapshot;

    #[test]
    fn tray_title_shows_amount_only_for_normal_snapshots() {
        let mut snapshot = build_loading_snapshot(&DashboardSettings::default());
        snapshot.total_cost_usd = 12.3456;

        assert_eq!(format_tray_title(&snapshot), "$12.35");
    }

    #[test]
    fn tray_title_hides_text_for_error_snapshots() {
        let snapshot = build_error_snapshot("timeout");

        assert_eq!(format_tray_title(&snapshot), "--");
    }

    #[test]
    fn loading_snapshot_preserves_dashboard_flags_from_settings() {
        let settings = DashboardSettings {
            always_on_top: true,
            current_provider: "claude".to_string(),
            enabled_providers: vec!["claude".to_string(), "kimi".to_string()],
        };

        let snapshot = build_loading_snapshot(&settings);

        assert_eq!(snapshot.provider_id, "claude");
        assert_eq!(
            snapshot.enabled_provider_ids,
            vec!["claude".to_string(), "kimi".to_string()]
        );
        assert!(snapshot.dashboard_always_on_top);
    }

    #[test]
    fn navigation_event_payload_serializes_expected_view_name() {
        let payload = serde_json::to_value(NavigateEvent {
            view: WindowView::Settings,
            reload_settings: Some(true),
        })
        .expect("navigate event should serialize");

        assert_eq!(
            payload,
            json!({
                "view": "settings",
                "reloadSettings": true,
            })
        );
    }

    #[test]
    fn copy_png_base64_to_clipboard_rejects_invalid_payload() {
        assert!(copy_png_base64_to_clipboard("not-base64").is_err());
    }

    #[test]
    fn copy_png_base64_to_clipboard_accepts_png_payload() {
        let mut bytes = Vec::new();
        let encoder = PngEncoder::new(&mut bytes);
        encoder
            .write_image(&[0, 0, 0, 255], 1, 1, ColorType::Rgba8.into())
            .expect("png should encode");
        let payload = STANDARD.encode(bytes);

        let result = copy_png_base64_to_clipboard(&payload);
        if let Err(error) = result {
            let message = format!("{error:#}");
            assert!(
                message.contains("failed to access system clipboard")
                    || message.contains("failed to copy image to clipboard")
            );
        }
    }
}
