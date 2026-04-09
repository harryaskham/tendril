# `tendril alias`

Use `tendril alias` to emit a transparent shell wrapper for a chosen target.

## Example

```bash
eval "$(tendril --window <id> alias --name desk)"
desk capture --json
desk run 'send("hello")'
```

## Why this exists

Tendril is intentionally stateless. Instead of storing a session target internally, it can generate shell code that pre-fills `--window` or `--display` while leaving later flags and subcommands under the caller's control.

## Current options

- `--name <alias-name>`
- `--shell <bash|zsh|fish|powershell>`

## Output modes

- human mode prints shell code directly,
- JSON mode returns the rendered command, argv, shell code, and target metadata.

That makes alias generation usable by both humans and agents without introducing hidden runtime state.
