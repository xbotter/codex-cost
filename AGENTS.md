# AGENTS.md

## Purpose

This file records repository-specific instructions for AI coding agents working in this project.

## Log Parsing Standard

- Treat local usage logs as partially corruptible input.
- Never fail an entire provider refresh because a single JSONL line is malformed.
- When a log line cannot be parsed as JSON, skip that line and continue processing the rest of the file.
- Prefer degraded partial results over a full refresh failure.
- Apply this standard consistently to every usage provider that reads line-oriented local logs.

## Error Handling

- Reserve hard failures for cases where the whole source cannot be read or the overall data source is unavailable.
- For per-line parse problems, prefer tolerant parsing plus optional diagnostics over surfacing a blocking user-facing error.
- When practical, track skipped lines so the UI can warn about partial data without hiding all usable results.
