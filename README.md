# codex-cost

A cross-platform Tauri tray app that reads local Codex session logs, estimates the daily USD cost, and keeps that value visible from the system tray.

## Why

`codex-cost` is meant for people who already use Codex locally and want a lightweight, always-on cost view without opening logs or running CLI reports manually.

## Acknowledgements

This project is inspired by [`ccusage`](https://github.com/ryoppippi/ccusage).

- The pricing source and overall Codex accounting direction were validated against `ccusage`.
- `codex-cost` focuses on a desktop tray experience and local always-on visibility rather than a CLI report workflow.

## Features

- Reads local Codex session JSONL logs directly
- Aggregates usage by local day and local timezone
- Includes subagent usage
- Uses online LiteLLM pricing with local caching
- Calculates billable input as `input_tokens - cached_input_tokens`
- Includes `reasoning_output_tokens` in output cost
- Shows cost in the tray and a compact dashboard
- Minimizes to tray and reopens on tray double-click

## Supported Platforms

- Windows
- macOS
- Linux

## Installation

Download a release artifact from the GitHub Releases page for your platform.

Windows builds are distributed as an NSIS installer. The installer also places `WebView2Loader.dll` next to the app binary to avoid missing-loader startup failures.

## Development

Requirements:

- Node.js 20+
- Rust stable
- Tauri v2 prerequisites for your platform

Install dependencies:

```bash
npm install
```

Run local checks:

```bash
npm run check
```

Run in development:

```bash
npm run tauri dev
```

Build release artifacts locally:

```bash
npm run build
npx tauri build
```

## How Usage Is Calculated

- Usage is tracked per session by reading `token_count.total_token_usage` snapshots.
- Costs are calculated from session deltas, not by summing raw cumulative values.
- Cross-day sessions are handled by preserving the prior session baseline before counting the first event of the current day.
- Pricing is fetched online and cached locally.

## Repository Standards

This repository includes:

- MIT license
- contribution guide
- code of conduct
- security policy
- issue and PR templates
- GitHub Actions for CI and release builds
- generated GitHub release notes configuration

## Release Process

Pushing a tag like `v0.1.0` triggers the release workflow. It builds release artifacts for Windows, macOS, and Linux, then publishes them to GitHub Releases with generated release notes.

## License

MIT. See [LICENSE](LICENSE).
