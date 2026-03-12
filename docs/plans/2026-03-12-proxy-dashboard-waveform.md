# Proxy Dashboard Waveform Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Port the `clashctl`-style waveform treatment into the proxy dashboard and slightly enlarge the dashboard card so the graph has room to breathe.

**Architecture:** Reuse the existing proxy request-delta history as the signal source, but render it with a local ratatui widget patterned after `George-Miao/clashctl`'s multi-line sparkline. Keep the rest of the dashboard copy and metrics intact, only changing the graph treatment and layout sizing.

**Tech Stack:** Rust, ratatui 0.30, existing TUI snapshot-style tests in `src-tauri/src/cli/tui/ui/tests.rs`

---

### Task 1: Lock down the desired dashboard behavior with tests

**Files:**
- Modify: `src-tauri/src/cli/tui/ui/tests.rs`

**Step 1: Write the failing test**
- Add a dashboard render test that expects the active proxy card to show the new braille-style waveform glyphs instead of the old single-line block sparkline.
- Add a focused waveform test that expects mirrored upper/lower dot glyphs from the same request history.

**Step 2: Run test to verify it fails**
- Run: `cargo test ui::tests::home_shows_proxy_dashboard_when_current_app_proxy_is_on ui::tests::proxy_activity_wave_uses_real_request_history`
- Expected: FAIL because the dashboard still renders the legacy single-line bar waveform.

### Task 2: Port the waveform widget and wire it into the dashboard

**Files:**
- Create: `src-tauri/src/cli/tui/ui/proxy_wave.rs`
- Modify: `src-tauri/src/cli/tui/ui.rs`
- Modify: `src-tauri/src/cli/tui/ui/main_page.rs`

**Step 1: Write minimal implementation**
- Add a local widget modeled on `clashctl/src/ui/components/sparkline.rs`, adapted to ratatui 0.30.
- Expose dot bar sets inspired by `clashctl/src/ui/components/traffic.rs`.
- Replace the old string-based graph rendering with a two-row mirrored waveform area and increase the proxy dashboard card height slightly.

**Step 2: Run targeted tests to verify it passes**
- Run: `cargo test ui::tests::home_shows_proxy_dashboard_when_current_app_proxy_is_on ui::tests::proxy_activity_wave_uses_real_request_history`
- Expected: PASS.

### Task 3: Verify the full affected surface

**Files:**
- Modify if needed: `src-tauri/src/cli/tui/ui/tests.rs`

**Step 1: Run broader verification**
- Run: `cargo test ui::tests::home_ ui::tests::proxy_activity_`
- Expected: PASS with no regressions in existing home/dashboard coverage.

**Step 2: Run formatting if code changed**
- Run: `cargo fmt`

**Step 3: Run final validation**
- Run: `cargo test ui::tests::home_ ui::tests::proxy_activity_`
- Expected: PASS after formatting.
