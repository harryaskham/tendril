# `tendril listen`

Use `tendril listen` to probe the current platform's audio-capture capability.

## Example

```bash
tendril listen --json --source system --duration-ms 5000 --format wav
```

## Current v0.0.1 scope

`listen` is intentionally probe-first in the current implementation:

- it accepts explicit source, duration, and format inputs,
- it reports backend capability and permission state, and
- it returns a structured note that audio artifact emission is not implemented yet.

## Supported source selectors

- `system`
- `loopback`
- `microphone`
- `device:<id>`

The explicit `device:<id>` form is modeled in the surface but currently returns a structured unsupported-capability result rather than binding to a real device.

## Why it is documented here

Even though the artifact path is not complete, the CLI surface and its current behavior are part of the repository's documented contract and should be visible in the published docs site.
