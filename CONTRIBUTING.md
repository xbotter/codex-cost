# Contributing

Thanks for contributing to `codex-cost`.

## Development Setup

1. Install Node.js 20+.
2. Install Rust stable.
3. Install the Tauri prerequisites for your platform.
4. Run:

```bash
npm install
```

## Local Checks

Run the standard verification command before opening a pull request:

```bash
npm run check
```

## Pull Requests

- Keep changes focused.
- Include context for user-visible behavior changes.
- Update documentation when behavior or setup changes.
- If the change affects release packaging, mention the target platforms you verified.

## Commit Style

Conventional-style commit subjects are preferred because they produce better generated release notes.

Examples:

- `feat: add claude provider scaffold`
- `fix: handle cross-day session baselines`
- `docs: update installation notes`
- `ci: add release workflow`

## Reporting Bugs

Please use the bug report template and include:

- platform and version
- how Codex is installed
- expected behavior
- actual behavior
- screenshots or logs if relevant
