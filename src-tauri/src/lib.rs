mod domain;
mod pricing;
mod providers;
mod service;

use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use arboard::{Clipboard, ImageData};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Local};
use domain::AppSnapshot;
use service::{
    billable_input_tokens, build_error_snapshot, format_token_count, total_output_tokens,
    UsageAppService,
};
use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::WindowEvent;
use tauri::{AppHandle, Emitter, Manager, RunEvent};

const TRAY_ID: &str = "usage-tray";
const MENU_STATUS: &str = "status";
const MENU_TOKENS: &str = "tokens";
const MENU_UPDATED: &str = "updated";
const MENU_REFRESH: &str = "refresh";
const MENU_REFRESH_PRICING: &str = "refresh_pricing";
const MENU_SHOW: &str = "show";
const MENU_QUIT: &str = "quit";

struct AppState {
    service: Arc<UsageAppService>,
    snapshot: Arc<Mutex<AppSnapshot>>,
    status_item: Arc<Mutex<Option<MenuItem<tauri::Wry>>>>,
    tokens_item: Arc<Mutex<Option<MenuItem<tauri::Wry>>>>,
    updated_item: Arc<Mutex<Option<MenuItem<tauri::Wry>>>>,
}

impl AppState {
    fn new() -> Self {
        let service = UsageAppService::new().expect("failed to initialize service");
        Self {
            service: Arc::new(service),
            snapshot: Arc::new(Mutex::new(build_loading_snapshot())),
            status_item: Arc::new(Mutex::new(None)),
            tokens_item: Arc::new(Mutex::new(None)),
            updated_item: Arc::new(Mutex::new(None)),
        }
    }
}

#[tauri::command]
fn get_snapshot(state: tauri::State<'_, AppState>) -> AppSnapshot {
    state.snapshot.lock().unwrap().clone()
}

#[tauri::command]
fn refresh_snapshot(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    force_pricing_refresh: bool,
) -> Result<AppSnapshot, String> {
    let current_snapshot = state.snapshot.lock().unwrap().clone();
    let service = state.service.clone();
    let snapshot_store = state.snapshot.clone();
    let status_item = state.status_item.clone();
    let tokens_item = state.tokens_item.clone();
    let updated_item = state.updated_item.clone();

    thread::spawn(move || {
        let snapshot = match service.refresh(force_pricing_refresh) {
            Ok(snapshot) => snapshot,
            Err(error) => build_error_snapshot(format!("{error:#}")),
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
                let _ = window.hide();
            }
            spawn_initial_refresh(app.handle().clone());
            spawn_periodic_refresh(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            refresh_snapshot,
            copy_dashboard_image_to_clipboard
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen {
                has_visible_windows,
                ..
            } = event
            {
                if !has_visible_windows {
                    show_dashboard(app);
                }
            }
        });
}

fn build_tray(app: &mut tauri::App<tauri::Wry>) -> tauri::Result<()> {
    let status_item = MenuItem::with_id(app, MENU_STATUS, "Today: --", false, None::<&str>)?;
    let tokens_item = MenuItem::with_id(app, MENU_TOKENS, "Tokens: --", false, None::<&str>)?;
    let updated_item = MenuItem::with_id(app, MENU_UPDATED, "Updated: --", false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let refresh_item = MenuItem::with_id(app, MENU_REFRESH, "Refresh now", true, None::<&str>)?;
    let refresh_pricing_item = MenuItem::with_id(
        app,
        MENU_REFRESH_PRICING,
        "Refresh pricing",
        true,
        None::<&str>,
    )?;
    let show_item = MenuItem::with_id(app, MENU_SHOW, "Open dashboard", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;

    let menu = MenuBuilder::new(app)
        .items(&[
            &status_item,
            &tokens_item,
            &updated_item,
            &separator,
            &refresh_item,
            &refresh_pricing_item,
            &show_item,
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
            MENU_REFRESH_PRICING => {
                let state = app.state::<AppState>();
                let snapshot = refresh_state(app, &state, true);
                let _ = emit_snapshot(app, &snapshot);
            }
            MENU_SHOW => {
                show_dashboard(app);
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
        let snapshot = match service.refresh(false) {
            Ok(snapshot) => snapshot,
            Err(error) => build_error_snapshot(format!("{error:#}")),
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
    let snapshot = match state.service.refresh(force_pricing_refresh) {
        Ok(snapshot) => snapshot,
        Err(error) => build_error_snapshot(format!("{error:#}")),
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
        item.set_text(format!("Today: ${:.2}", snapshot.total_cost_usd))?;
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

fn build_loading_snapshot() -> AppSnapshot {
    let now = Local::now();

    AppSnapshot {
        provider_id: "codex".to_string(),
        date: now.date_naive().to_string(),
        title: "--".to_string(),
        tooltip: "Loading usage…".to_string(),
        total_cost_usd: 0.0,
        total_cost_sparkline: vec![0.0; 48],
        totals: Default::default(),
        model_costs: Vec::new(),
        pricing_updated_at: None,
        used_stale_pricing: false,
        last_refreshed_at: now.to_rfc3339(),
        error_message: None,
    }
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

fn show_dashboard(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
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

    use super::{build_loading_snapshot, copy_png_base64_to_clipboard, format_tray_title};
    use crate::service::build_error_snapshot;

    #[test]
    fn tray_title_shows_amount_only_for_normal_snapshots() {
        let mut snapshot = build_loading_snapshot();
        snapshot.total_cost_usd = 12.3456;

        assert_eq!(format_tray_title(&snapshot), "$12.35");
    }

    #[test]
    fn tray_title_hides_text_for_error_snapshots() {
        let snapshot = build_error_snapshot("timeout");

        assert_eq!(format_tray_title(&snapshot), "--");
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
