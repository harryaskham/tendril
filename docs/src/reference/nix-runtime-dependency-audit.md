# Nix runtime dependency audit

Tendril's Linux Nix package uses wrapper scripts to put platform helper tools on `PATH`. The package should expose only commands Tendril actually executes, not whole dependency `bin/` directories. This keeps the wrapper deterministic and reduces the chance that a downstream profile or `buildEnv` accidentally inherits unrelated tools from Tendril's helper dependencies.

## Direct runtime commands

The `tendril` wrapper exposes these Linux desktop helpers:

| Command | Source package | Why Tendril needs it |
| --- | --- | --- |
| `hyprctl` | `hyprland` | Hyprland window/compositor discovery. |
| `swaymsg` | `sway` | Sway/i3-compatible window discovery. |
| `wlr-randr` | `wlr-randr` | Wayland output/display discovery fallback. |
| `grim` | `grim` | Wayland screenshot fallback. |
| `ydotool` | `ydotool` | Wayland input injection fallback. |
| `wtype` | `wtype` | Wayland text/key input fallback. |

## Headless helper commands

The `tendril-headless` wrapper also exposes the commands used by `scripts/tendril-headless.sh`: shell/coreutils helpers (`bash`, `sh`, `basename`, `cat`, `chmod`, `dirname`, `env`, `mkdir`, `mktemp`, `rm`, `sleep`, `tail`, and `tr`), `grep`, `chromium`, `firefox`, `openbox`, `python3`, `xdpyinfo`, `xsetroot`, `xterm`, and `Xvfb`.

## Collision findings

- The audited Tendril wrapper command set does **not** include `ss` and does not include `iproute2` directly.
- The previously reported `agent-utils` versus `iproute2` conflict is a host profile / system package composition conflict, not a Tendril package output conflict.
- The Nix check `linuxRuntimeDependencyAudit` records the wrapper command list and fails if two declared helper packages try to expose the same command name through Tendril's curated runtime path.

## Policy for future dependencies

When adding a Linux helper dependency, add a single command entry in `flake.nix` instead of adding the whole package to a wrapper `PATH`. If Tendril needs multiple binaries from the same package, list each one explicitly so reviewers can see the installed-path surface and spot possible overlaps before a NixOS or home-manager rebuild fails.
