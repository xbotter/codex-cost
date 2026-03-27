# Single-Window Navigation Design

## Summary

Unify the `Dashboard` and `Settings` experiences into a single application window.

The app keeps one primary webview window (`main`) and switches between two in-window views:

- `Dashboard`
- `Settings`

This replaces the current dedicated `Settings` window model. Tray actions, toolbar actions, and secondary app launches all resolve to the same main window so the app behaves like a single-window utility instead of a multi-window tray app.

## Goals

- Merge `Dashboard` and `Settings` into one window
- Keep the interaction model simple: one app instance, one content window
- Route every "open settings" action to the existing main window
- Preserve current quota/provider settings behavior
- Keep the app feeling native on macOS by avoiding unnecessary utility windows

## Non-Goals

- Redesigning the dashboard information hierarchy
- Adding new settings categories beyond the current provider/quota settings
- Introducing a tab bar or sidebar for future navigation expansion
- Reworking tray-first behavior or background refresh logic

## Product Behavior

### Window Model

The app has one primary content window: `main`.

There is no separate `settings` webview window after this change.

If the user:

- clicks the tray `Open dashboard` item
- clicks the tray `Settings` item
- clicks the in-app settings button
- launches the app again while it is already running
- reopens the app from the dock while no window is visible

the app reuses the existing `main` window.

### View Model

The main window supports two views:

- `dashboard`
- `settings`

`dashboard` remains the default startup view.

Opening Settings does not create a new window. It:

1. shows the `main` window if hidden
2. focuses it
3. switches the in-window view to `settings`
4. reloads persisted settings into the form so stale unsaved edits are discarded

Returning to Dashboard switches the current view back to `dashboard` in the same window.

### Settings Behavior

The Settings screen keeps the current provider list and provider detail editing flow:

- provider enable/disable
- provider quota mode
- provider quota amount

Saving settings continues to:

- validate enabled providers and quota amounts
- persist dashboard settings
- persist provider quota settings
- refresh in-memory snapshot state
- update the visible dashboard data

If the user opens Settings, makes unsaved edits, leaves Settings, and opens Settings again from a global entry point, the form reloads persisted data instead of preserving the stale draft.

## UX Design

### Dashboard View

The current dashboard layout remains the primary landing view.

Its top-right action area keeps a Settings affordance, but that affordance now means "switch this window to the Settings view" instead of "open another window."

### Settings View

The Settings view is embedded into the main document shell and visually harmonized with the dashboard surface.

Recommended structure:

- compact top bar with back action and title
- provider list on the left at larger widths
- provider detail/editor on the right
- stacked layout on narrower widths if needed

The Save action remains explicit. After successful save, the view can stay on Settings and show success feedback; it does not need to auto-navigate back.

### Tray and App-Level Commands

Tray labels can stay unchanged for now:

- `Open dashboard`
- `Settings`

Behavior changes only in routing:

- `Open dashboard` shows `main` and selects `dashboard`
- `Settings` shows `main` and selects `settings`

This keeps the tray familiar while making the window model consistent.

## Data Flow

1. App starts and creates only the `main` window from Tauri config
2. Frontend boots in `dashboard` view
3. Global actions emit or invoke view-switch behavior instead of creating windows
4. When switching to `settings`, frontend reloads current persisted settings
5. User edits and saves
6. Backend persists dashboard/quota settings
7. Backend refreshes snapshot state and emits snapshot updates
8. Frontend dashboard view reflects new state immediately when shown

## Implementation Shape

### Backend

Backend window management should move from "show dashboard" and "show settings window" to "show main window and select a view."

Recommended changes:

- keep `show_dashboard(app)` but make it also request the `dashboard` view
- replace dedicated settings window creation with a function that shows `main` and requests the `settings` view
- remove use of `WebviewWindowBuilder` for `settings.html`
- keep single-instance callback focused on reusing `main`

The single-instance plugin is already part of the app dependencies and should remain enabled so repeated launches focus the current app instead of opening a second process.

### Frontend

The frontend should consolidate the current `main.ts` and `settings.ts` behaviors under one page shell.

Recommended split:

- one shared HTML document: `index.html`
- one top-level app script that owns current view state
- settings rendering/helpers extracted into reusable functions or a small module

The app should respond to a backend-emitted navigation event such as a `navigate` or `view-change` event with payload:

```json
{
  "view": "dashboard"
}
```

or:

```json
{
  "view": "settings"
}
```

The frontend should also support local switching from the top-right Settings button and an in-view Back button.

### Assets and Entry Points

After consolidation:

- `settings.html` should no longer be used
- the separate settings window script/style entry points should either be removed or reduced to imported modules used by the main page

This avoids two drifting implementations of the same settings UI.

## Validation Rules

- At least one provider must remain enabled
- Enabled providers with quota enabled must have a positive numeric USD amount
- Switching to Settings from tray or toolbar always reloads persisted settings
- Saving settings refreshes the dashboard snapshot without requiring an app restart
- Repeated app launches focus the existing instance instead of opening a second process
- No code path should create a `settings` webview window

## Testing Strategy

### Automated

- frontend build succeeds after the page consolidation
- Rust check succeeds after window-management changes
- existing settings/domain tests continue to pass
- add or update unit tests for any backend helper that now routes to views instead of creating a second window where practical

### Manual

1. Launch app, open dashboard from tray
2. Open Settings from tray and confirm the same window switches views
3. Open Settings from the in-app button and confirm same behavior
4. Edit settings without saving, leave Settings, reopen from tray, confirm form reloads persisted values
5. Save settings and confirm dashboard values refresh
6. Launch the app again while it is already running and confirm the existing window is focused
7. Close the window, reopen from dock/tray, confirm the same window returns

## Risks

### Frontend Consolidation Risk

The current dashboard and settings pages were built as separate entry points with different styling assumptions. Folding them into one shell can create CSS collisions or layout regressions if styles are merged carelessly.

Mitigation:

- keep settings styles namespaced
- preserve the existing dashboard structure
- prefer a clear root class per view

### Event Routing Risk

Moving from window creation to in-window view switching changes how tray and startup flows reach Settings.

Mitigation:

- keep backend routing minimal and explicit
- use one event channel for view selection
- verify every global entry point manually

## Migration Notes

This design intentionally supersedes the dedicated settings window decision documented in [2026-03-18-quota-settings-design.md](/Users/xbotter/Code/App/codex-cost/docs/superpowers/specs/2026-03-18-quota-settings-design.md).

Quota settings remain part of the product, but the window model changes from:

- dedicated `Settings` window

to:

- single `main` window with in-place navigation
