# Quota Settings Design

## Summary

Add a daily USD quota feature to `codex-cost` so users can track progress against either:

- a daily spending `Target`
- a daily spending `Cap`

The quota is configured from a dedicated `Settings` window opened from the tray menu. The main dashboard remains read-focused and only surfaces the current quota state.

## Goals

- Let users define a daily quota in USD cost
- Support both "goal to reach" and "limit not to exceed" semantics
- Keep the main dashboard visually quiet
- Keep configuration outside the dashboard, behind tray `Settings`
- Reuse the current tray-first application model

## Non-Goals

- Token-based quotas
- Multiple quotas per provider or per model
- Historical quota analytics
- Notifications or alerts beyond the dashboard state
- Rich preferences management for unrelated app settings

## Product Behavior

### Quota Model

Quota uses a minimal persisted shape:

- `enabled: boolean`
- `mode: "target" | "cap"`
- `amount_usd: number`

Quota is global for the app and applies to the currently supported provider flow.

### Daily Boundary

Quota uses the same local-day boundary as the existing daily usage snapshot.

That means:

- quota reset follows the app's local timezone
- no separate timezone setting is introduced
- quota progress is always based on the same `today` definition as daily cost

### Dashboard Presentation

Use the selected `A1` layout: a compact inline quota row below the hero amount.

When quota is disabled:

- no quota row is shown

When quota is enabled in `Target` mode:

- left text: `Target $250`
- right text: `74% reached`
- progress = `today_cost / amount_usd`
- progress is clamped visually to `100%`

When quota is enabled in `Cap` mode:

- left text: `Cap $250`
- right text: `$65.78 left`
- progress = `today_cost / amount_usd`
- if remaining is negative, right text becomes `Over by $xx.xx`
- progress is clamped visually to `100%` once the cap is met or exceeded

### Failure-State Behavior

Quota must not show synthetic progress when the underlying usage snapshot is in an error state.

If daily usage fails to load:

- the quota row remains visible when quota is enabled
- left text still reflects the configured mode, such as `Target $250` or `Cap $250`
- right text becomes `Unavailable`
- the progress bar should render as neutral or empty, not computed from fake zeroes

### Settings Entry Point

Tray menu gains a new `Settings` item.

Clicking `Settings` opens a small dedicated settings window. This avoids pushing form controls into the dashboard or trying to do numeric input directly inside native tray menus.

## UI Design

### Dashboard

Quota appears as a single compact section:

- small left label/value pair
- thin progress bar
- short right-side status text

The row should feel secondary to the main daily cost number. It should not become a second hero block.

### Settings Window

The settings window contains only quota controls in the first version:

- `Enable quota` toggle
- `Mode` segmented choice:
  - `Target`
  - `Cap`
- `Daily USD amount` numeric input
- `Save` action

Recommended behavior:

- small utility window
- opens centered
- not shown in taskbar if avoidable
- close hides the window instead of exiting the app
- opening the window reloads persisted values into the form
- closing without saving discards unsaved edits
- reopening always shows last persisted settings, not hidden stale form state

## Data Flow

1. App loads settings from persisted local config on startup
2. Snapshot service reads current quota settings when computing dashboard state
3. Frontend receives quota state as part of the existing snapshot payload
4. Dashboard renders quota row only when enabled
5. User opens `Settings` from tray
6. Settings window loads current quota config
7. User edits and saves
8. Backend persists config locally
9. Backend refreshes in-memory state and emits updated snapshot
10. Dashboard reflects the new quota state immediately

If the settings window is reopened without saving:

- the form is rehydrated from persisted settings
- unsaved hidden edits are not restored

## Persistence

Persist quota settings in local app config using the platform-appropriate app data directory already used by the Tauri backend.

Recommended structure:

```json
{
  "quota": {
    "enabled": true,
    "mode": "target",
    "amount_usd": 250.0
  }
}
```

Requirements:

- tolerate missing config file
- tolerate partial config
- validate negative or zero amounts
- fall back to disabled quota on invalid data
- write config atomically using write-then-replace behavior or equivalent
- update in-memory state only after a successful persisted write
- keep persistence safe with background refresh activity

## Backend Changes

### Domain

Add a quota settings domain model and a dashboard-facing quota presentation model.

Suggested split:

- persisted config model
- validated runtime settings model
- rendered snapshot quota model

### Service

Extend the snapshot service to compute:

- whether quota is enabled
- mode
- configured amount
- current progress ratio
- current display string for the right side
- whether quota state is derivable from a healthy daily usage snapshot

The service owns the quota semantics so frontend rendering stays simple.

### Tauri Commands

Add commands for:

- reading current settings
- saving quota settings

These commands must be explicitly available to the settings window capability scope.

### Window Management

Add a dedicated settings window with these properties:

- created lazily on first open
- reused on subsequent opens
- closed via hide behavior
- explicitly wired into Tauri v2 capability configuration so window commands work

### Tray

Add a `Settings` menu item near the existing app actions.

## Frontend Changes

### Main Dashboard

Extend the snapshot type with quota data:

- `enabled`
- `mode`
- `amount_usd`
- `progress_ratio`
- `primary_label`
- `status_label`

Render the compact quota row only when enabled.

### Settings UI

Create a separate frontend entry for settings or a second page/window binding, depending on the existing Tauri structure selected during implementation.

The form should:

- load current quota state on open
- validate amount before save
- disable amount input when quota is off only if the interaction still feels clear
- discard unsaved hidden state on reopen by reloading persisted values

## Validation Rules

- amount must be a finite positive number
- amount should be canonicalized to USD with two-decimal precision using half-up rounding
- values that round to `0.00` are invalid
- blank or invalid input should block save and show a compact inline error

## Edge Cases

- `amount_usd <= 0`: treat as invalid and refuse save
- `today_cost = 0`: render `0% reached` for target, full remaining amount for cap
- `today_cost > amount_usd` in cap mode: render `Over by $xx.xx`
- usage snapshot error: render `Unavailable`, not derived quota math
- very small amounts: keep formatting stable to 2 decimals
- quota disabled after previously enabled: dashboard row disappears immediately
- local midnight rollover resets quota progress on the same local-day boundary as daily cost

## Testing

### Backend

- load config when missing
- reject invalid quota amounts
- compute target progress and `reached` text correctly
- compute cap remaining and `Over by` text correctly
- snapshot excludes quota row when disabled
- snapshot renders quota safely when usage is unavailable
- rounding and `0.00` rejection behave correctly
- persistence writes are atomic and reload safely

### Frontend

- dashboard renders no quota section when disabled
- dashboard renders correct target text
- dashboard renders correct cap text
- dashboard renders `Unavailable` on snapshot errors without fake progress
- settings form loads existing values
- save updates dashboard after backend event
- settings window has the required command capability access

### Manual

- enable target quota, save, verify dashboard
- enable cap quota, save, verify dashboard
- exceed cap and verify overage copy
- reopen settings and confirm persistence
- app restart preserves settings
- close settings with unsaved edits, reopen, verify persisted values reload
- cross local midnight and verify quota resets with daily totals

## Risks

- Adding a second window introduces extra Tauri window lifecycle paths
- If quota formatting logic lives in both frontend and backend, behavior can drift
- Tray-first apps can feel fragile if settings window handling is not consistent

## Recommendation

Proceed with:

- global USD-only quota
- `Target` and `Cap` modes
- compact `A1` dashboard row
- dedicated `Settings` window launched from tray

This keeps the feature small, understandable, and aligned with the product's current quiet dashboard style.
