# Quota Settings Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a daily USD quota feature with `Target` and `Cap` modes, shown as a compact dashboard row and configured from a dedicated tray-launched settings window.

**Architecture:** Keep quota semantics in the Rust backend so the frontend only renders precomputed labels and progress state. Persist quota settings in the existing app data area, extend the snapshot contract with quota presentation fields, and add a second Tauri window plus capability scope for settings management.

**Tech Stack:** Tauri v2, Rust, serde JSON persistence, existing TypeScript dashboard frontend, native tray menu, Tauri multi-window capabilities.

---

## File Map

**Backend domain and service**

- Modify: `src-tauri/src/domain.rs`
  - Add persisted/runtime/rendered quota structs
  - Extend `AppSnapshot`
- Modify: `src-tauri/src/service.rs`
  - Load quota settings
  - Compute target/cap presentation state
  - Handle error snapshots safely
- Create: `src-tauri/src/settings.rs`
  - Read/write quota config
  - Atomic persistence helpers

**Tauri app shell**

- Modify: `src-tauri/src/lib.rs`
  - Add settings window management
  - Add tray `Settings` item
  - Add commands for reading/saving quota settings
- Modify: `src-tauri/src/main.rs`
  - Only if module wiring requires it
- Modify: `src-tauri/tauri.conf.json`
  - Register settings window metadata if static config is used
- Create: `src-tauri/capabilities/settings.json`
  - Allow commands from the settings window
- Modify: `src-tauri/capabilities/default.json`
  - Keep `main` scoped cleanly

**Frontend**

- Modify: `src/main.ts`
  - Render quota row in dashboard
  - Respect error-state quota rendering
- Modify: `src/styles.css`
  - Add compact A1 quota row styles
- Modify: `index.html`
  - Insert dashboard quota container
- Create: `src/settings.ts`
  - Settings window frontend
- Create: `settings.html`
  - Settings window document shell
- Create: `src/settings.css`
  - Minimal settings window styles if needed

**Tests**

- Modify: `src-tauri/src/domain.rs`
  - Domain serialization/unit tests
- Modify: `src-tauri/src/service.rs`
  - Service quota logic tests
- Add frontend checks only if existing harness supports them without introducing a new test framework

---

## Chunk 1: Quota Domain and Persistence

### Task 1: Add quota domain models

**Files:**
- Modify: `src-tauri/src/domain.rs`
- Test: `src-tauri/src/domain.rs`

- [ ] **Step 1: Add failing Rust tests for quota serialization and basic validation shapes**

Add tests for:
- disabled default quota state
- valid `target` / `cap` serde round-trip
- snapshot quota payload serde shape

- [ ] **Step 2: Run the domain-focused test target**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml domain
```

Expected:
- new quota tests fail because types do not exist yet

- [ ] **Step 3: Add quota domain types**

Implement:
- persisted quota settings struct
- validated runtime quota settings struct
- rendered snapshot quota struct with fields like:
  - `enabled`
  - `mode`
  - `amount_usd`
  - `progress_ratio`
  - `primary_label`
  - `status_label`
  - `is_error_state`

- [ ] **Step 4: Extend `AppSnapshot` with optional quota presentation**

Keep it optional so disabled quota does not force fake values through the frontend.

- [ ] **Step 5: Re-run the domain tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml domain
```

Expected:
- quota domain tests pass

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/domain.rs
git commit -m "feat: add quota domain models"
```

### Task 2: Implement quota config persistence

**Files:**
- Create: `src-tauri/src/settings.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/settings.rs`

- [ ] **Step 1: Write failing tests for config load/save behavior**

Cover:
- missing config returns disabled quota
- partial config falls back safely
- invalid amount disables quota or rejects save
- atomic write path produces final valid JSON

- [ ] **Step 2: Run the settings-focused tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml settings
```

Expected:
- tests fail because settings module does not exist yet

- [ ] **Step 3: Implement config path resolution and atomic write helper**

Use:
- app data directory already used by the backend
- write-to-temp then rename/replace strategy

- [ ] **Step 4: Implement quota load/save functions**

Rules:
- half-up round to two decimals before persistence
- reject values that round to `0.00`
- only update in-memory state after successful write

- [ ] **Step 5: Re-run settings tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml settings
```

Expected:
- settings tests pass

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/settings.rs src-tauri/src/lib.rs
git commit -m "feat: persist quota settings"
```

---

## Chunk 2: Snapshot and Quota Semantics

### Task 3: Compute quota presentation in the service layer

**Files:**
- Modify: `src-tauri/src/service.rs`
- Modify: `src-tauri/src/domain.rs`
- Test: `src-tauri/src/service.rs`

- [ ] **Step 1: Add failing service tests for target and cap modes**

Cover:
- disabled quota returns no quota payload
- target mode renders `Target $250` and `74% reached`
- cap mode renders `Cap $250` and `$65.78 left`
- cap overrun renders `Over by $xx.xx`
- usage error renders `Unavailable`
- local-day rollover behavior uses existing daily snapshot date

- [ ] **Step 2: Run the service-focused tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml service
```

Expected:
- new quota tests fail

- [ ] **Step 3: Implement quota presentation builder**

Keep all semantics in Rust:
- current amount
- progress ratio
- primary label
- status label
- error-safe fallback state

- [ ] **Step 4: Thread quota state into normal and error snapshots**

Ensure:
- error snapshots do not compute fake progress from zero
- disabled quota stays absent

- [ ] **Step 5: Re-run service tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml service
```

Expected:
- quota service tests pass

- [ ] **Step 6: Run a broader Rust verification pass**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected:
- full Rust test suite passes, or environment-specific blockers are documented

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/domain.rs src-tauri/src/service.rs
git commit -m "feat: compute quota snapshot state"
```

---

## Chunk 3: Tauri Windows, Commands, and Capabilities

### Task 4: Add settings window and tray entry

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tauri.conf.json`
- Create: `src-tauri/capabilities/settings.json`
- Modify: `src-tauri/capabilities/default.json`

- [ ] **Step 1: Add failing tests or minimal verification hooks for settings window management**

At minimum, define assertions around:
- tray menu includes `Settings`
- settings window open path is callable
- close hides instead of quitting

- [ ] **Step 2: Add a settings tray menu item and open handler**

Keep `main` dashboard behavior unchanged.

- [ ] **Step 3: Implement lazy settings window creation/reuse**

Requirements:
- reuse same label each time
- opening reloads persisted settings
- closing hides window

- [ ] **Step 4: Add Tauri commands for get/save quota settings**

Expose:
- `get_quota_settings`
- `save_quota_settings`

- [ ] **Step 5: Add capability wiring for the settings window**

Create a settings capability that allows:
- `core:default`
- required invoke permissions for new commands

- [ ] **Step 6: Verify capability JSON and app boot**

Run:

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected:
- capability config accepted
- no window/command registration errors

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/tauri.conf.json src-tauri/capabilities/default.json src-tauri/capabilities/settings.json
git commit -m "feat: add quota settings window"
```

---

## Chunk 4: Dashboard Quota Row

### Task 5: Render compact quota row on the dashboard

**Files:**
- Modify: `index.html`
- Modify: `src/main.ts`
- Modify: `src/styles.css`

- [ ] **Step 1: Add the dashboard quota container markup**

Place it below the hero amount and before the metrics section.

- [ ] **Step 2: Update frontend snapshot typing**

Add typed quota payload fields without duplicating backend formatting logic.

- [ ] **Step 3: Render target/cap/error variants**

Rules:
- no quota row when disabled
- `Target $250` + `74% reached`
- `Cap $250` + `$65.78 left`
- `Unavailable` on error snapshot

- [ ] **Step 4: Add compact A1 row styles**

Keep it visually secondary:
- thin progress bar
- small labels
- no second hero card

- [ ] **Step 5: Run frontend verification**

Run:

```bash
npm run check:frontend
```

Expected:
- TypeScript and frontend checks pass

- [ ] **Step 6: Commit**

```bash
git add index.html src/main.ts src/styles.css
git commit -m "feat: show quota progress on dashboard"
```

---

## Chunk 5: Settings Window Frontend

### Task 6: Build the quota settings form

**Files:**
- Create: `settings.html`
- Create: `src/settings.ts`
- Create or Modify: `src/settings.css`
- Modify: `package.json` only if an additional frontend entry needs explicit build wiring

- [ ] **Step 1: Add settings window HTML shell**

Include:
- enable toggle
- target/cap segmented control
- amount input
- save button
- compact inline validation area

- [ ] **Step 2: Implement settings window bootstrap**

On open:
- load persisted settings via command
- populate form state

On save:
- validate locally
- invoke backend save
- close or hide on success

- [ ] **Step 3: Implement reopen-safe form reset**

Ensure hidden unsaved state does not survive reopen.

- [ ] **Step 4: Style the settings window**

Direction:
- quiet utility window
- consistent with current dashboard palette
- simpler than the main dashboard

- [ ] **Step 5: Run frontend verification**

Run:

```bash
npm run check:frontend
```

Expected:
- settings entry compiles cleanly

- [ ] **Step 6: Manual verify settings flow**

Run:

```bash
npm run tauri dev
```

Verify:
- tray `Settings` opens the new window
- values persist
- dashboard updates after save
- close without save discards edits

- [ ] **Step 7: Commit**

```bash
git add settings.html src/settings.ts src/settings.css package.json package-lock.json
git commit -m "feat: add quota settings form"
```

---

## Chunk 6: End-to-End Verification and Release Readiness

### Task 7: Run regression checks

**Files:**
- Modify only if fixes are needed from verification

- [ ] **Step 1: Run Rust verification**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected:
- tests pass, or environment-specific limitations are documented precisely

- [ ] **Step 2: Run frontend verification**

```bash
npm run check:frontend
```

Expected:
- success

- [ ] **Step 3: Run packaging-oriented local smoke check**

```bash
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected:
- production frontend builds
- backend still checks with quota changes included

- [ ] **Step 4: Manual product verification**

Verify all of:
- target mode progress copy
- cap mode remaining copy
- cap overage copy
- disabled quota hides row
- errored snapshot shows `Unavailable`
- quota resets on local-day transition assumptions
- settings window capability works

- [ ] **Step 5: Commit final fixes**

```bash
git add .
git commit -m "fix: finalize quota settings flow"
```

---

## Notes for Execution

- Keep quota formatting logic in Rust, not duplicated in TypeScript.
- Do not introduce token quotas or notifications in this pass.
- Reuse current tray-first architecture; do not turn dashboard into a settings surface.
- Be careful with Tauri v2 capabilities. The second window is the easiest place to create a runtime regression.
- Prefer adding focused tests into existing Rust modules over introducing a new test harness.

Plan complete and saved to `docs/superpowers/plans/2026-03-18-quota-settings.md`. Ready to execute?
