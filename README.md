# ACX — AutoContinue + Codex

<p align="center"><code>npm i -g @moyeranqianzhi/acx</code></p>
<p align="center"><strong>ACX</strong> is an enhanced version of <a href="https://github.com/openai/codex">OpenAI Codex CLI</a> with built-in <a href="https://github.com/MoYeRanqianzhi/AutoContinue">AutoContinue</a> auto-continue and auto-retry capabilities.</p>
<p align="center"><a href="README.zh-CN.md">中文文档</a></p>

---

## What is ACX?

ACX = **A**uto**C**ontinue + Code**X**

Built on top of [OpenAI Codex CLI](https://github.com/openai/codex), ACX embeds the core logic of [AutoContinue](https://github.com/MoYeRanqianzhi/AutoContinue) to provide:

- **Auto-Continue**: Automatically sends a continue prompt after each turn completes — no manual intervention needed
- **Smart Retry**: Retries with exponential backoff on transient errors (rate limits, server overload, connection failures, etc.), capped at 5 minutes
- **Auto-Stop on Fatal Errors**: Permanent errors like context exceeded or auth failure stop immediately without wasting retries
- **Extensible Stop Hooks**: Stop based on round count, wall-clock time, duration, or custom commands
- **`/acx-stop` Command**: Manually stop auto-continue at any time from the TUI

All original Codex CLI features are fully preserved.

## Installation

```shell
npm install -g @moyeranqianzhi/acx
```

The CLI binary is still named `codex` after installation.

## Usage

```shell
# Enable auto-continue mode
codex --acx "your task description"

# Custom continue prompt and delay
codex --acx --acx-cp "Keep iterating" --acx-delay 10 "refactor the project"

# With stop conditions
codex --acx --acx-sw "<round=5>" --acx-sw "<duration=3600>" "optimize the codebase"

# Stop manually during a session: type /acx-stop in the TUI
```

### ACX Flags

| Flag | Description | Default |
|------|-------------|---------|
| `--acx` | Enable auto-continue mode | `false` |
| `--acx-cp` | Continue prompt text | `"Continue"` |
| `--acx-cpio` | Continue prompt IO file (re-read each time) | — |
| `--acx-cpp` | Continue prompt pipe command | — |
| `--acx-delay` | Base delay in seconds (exponential backoff on errors) | `15` |
| `--acx-sw` | Preset stop condition (repeatable) | — |
| `--acx-sh` | Custom stop hook command (repeatable) | — |

### Stop Condition Examples

```shell
--acx-sw "<round=10>"       # Stop after 10 rounds
--acx-sw "<error>"           # Stop on any error (no retry)
--acx-sw "<duration=3600>"   # Stop after 1 hour
--acx-sw "<time=2026-01-01T08:00:00>"  # Stop at a specific time
```

## Supported Platforms

| Platform | Architecture | Status |
|----------|-------------|--------|
| Linux | x86_64, ARM64 | ✅ |
| macOS | ARM64 (Apple Silicon) | ✅ |
| Windows | x86_64, ARM64 | ✅ |

## Upstream Sync

ACX automatically syncs with [openai/codex](https://github.com/openai/codex) upstream updates every hour via GitHub Actions. ACX modifications follow a minimal-insertion principle (marked with `// [ACX]` comments), with core logic encapsulated in a standalone `codex-auto-continue` crate to minimize merge conflicts.

## Acknowledgements

- [OpenAI Codex CLI](https://github.com/openai/codex) — The underlying CLI framework, licensed under Apache-2.0
- [AutoContinue](https://github.com/MoYeRanqianzhi/AutoContinue) — The original auto-continue/retry logic

## License

This project is licensed under the [Apache-2.0 License](LICENSE), consistent with upstream Codex.

## Documentation

- [Codex Documentation](https://developers.openai.com/codex)
- [AutoContinue Project](https://github.com/MoYeRanqianzhi/AutoContinue)
- [Contributing](./docs/contributing.md)
- [Installing & building](./docs/install.md)
