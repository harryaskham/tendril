# Remote, WSL, and host tunnelling

Tendril keeps remote control stateless: the local process proxies the same CLI
arguments to another Tendril process and streams or forwards the result.

## SSH remote mode

```bash
tendril --remote user@host --json list
tendril --remote user@host --window <id> capture --json
tendril --remote user@host --window <id> run 'send("hello")'
```

`--remote user@host` strips only the local `--remote` flag, then executes
`tendril` on the remote host over OpenSSH. The remote host must have `tendril`
on `PATH`, or `TENDRIL_REMOTE_BIN` must name the executable.

On Linux remotes, the bootstrap handles common non-login-shell problems:

- adds common package-manager paths,
- ignores SSH X-forwarding displays that point back at the client,
- discovers `XDG_RUNTIME_DIR`, the session bus, `WAYLAND_DISPLAY`, and
  `DISPLAY` when the SSH environment did not inherit them, and
- exports `TENDRIL_DISCOVERED_WAYLAND_SOCKET` or
  `TENDRIL_DISCOVERED_X11_SOCKET` when it inferred a socket path.

For MCP stdio, remote mode streams stdin/stdout/stderr so framed JSON-RPC
messages pass through unchanged. For normal JSON commands, SSH failures are
wrapped in structured Tendril error envelopes.

## WSL tunnel mode

```bash
tendril --wsl-tunnel --json list
tendril --wsl-tunnel --window <hwnd> capture --json
tendril --remote wslbox --wsl-tunnel --json list
```

`--wsl-tunnel` proxies the invocation from WSL/Linux to a Windows-host
`tendril.exe`. It strips only `--wsl-tunnel` and preserves the rest of the CLI
arguments, so JSON/MCP behaviour is the same as a direct Windows Tendril run.

By default the tunnel first tries `TENDRIL_WSL_WINDOWS_BIN`, then
`tendril.exe` on the WSL-visible Windows PATH. If neither exists, Tendril
bootstraps the Windows side by downloading the latest Windows release asset from
GitHub, verifying the published `.sha256`, and installing `tendril.exe` under
`%LOCALAPPDATA%\\Tendril\\bin` (converted to its WSL mount path). The installed
binary is reused while its `tendril.version` marker matches the latest release.

Set `TENDRIL_WSL_WINDOWS_BIN` when the Windows binary lives somewhere else from
the WSL environment's point of view. Set `TENDRIL_WSL_INSTALL_DIR` to override
the auto-install directory, `TENDRIL_WSL_WINDOWS_RELEASE_VERSION` to pin a
release for bootstrap, `TENDRIL_WSL_WINDOWS_TARGET` to override the default
`x86_64-windows` asset, or `TENDRIL_WSL_WINDOWS_REPOSITORY` to download from a
fork.

## Composition model

`--remote` is evaluated first. That means:

```bash
tendril --remote user@linux-or-wsl-host --wsl-tunnel --json list
```

connects to the remote host, then asks the remote Tendril process to perform the
WSL-to-Windows hop. This supports a remote Linux controller targeting a Windows
host through a WSL environment while preserving Tendril's standard JSON output.
