{
  description = "Tendril Rust workspace and Nix development environment";

  inputs = {
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    mcp-cli = {
      url = "github:harryaskham/mcp-cli/941015b41aeb71a1af7efafc4d66727a93048696";
      flake = false;
    };
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self
    , crane
    , flake-utils
    , mcp-cli
    , nixpkgs
    ,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        lib = pkgs.lib;
        workspaceManifest = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        workspaceVersion = workspaceManifest.workspace.package.version;
        repositoryUrl = "https://github.com/harryaskham/tendril";
        craneLib = crane.mkLib pkgs;
        parentSrc = craneLib.cleanCargoSource ./.;
        fullParentSrc = lib.cleanSource ./.;
        # Keep the vendored workspace crate and Cargo's git source identity in
        # sync. Do not replace the downstream mcp-cli git dependency with a
        # root [patch] path entry without also testing the crane cargoArtifacts
        # build: crane-cleaned/grafted sources can rewrite patch tables so a
        # local Cargo build succeeds while Nix resolves updatable-cli against
        # the wrong mcp-cli API. See docs/src/mcp.md.
        graftMcpCli = name: baseSrc: pkgs.runCommand name { } ''
          cp -r ${baseSrc} "$out"
          chmod -R u+w "$out"
          rm -rf "$out/crates/mcp-cli"
          mkdir -p "$out/crates"
          cp -r ${mcp-cli} "$out/crates/mcp-cli"
          chmod -R u+w "$out/crates/mcp-cli"
        '';
        src = graftMcpCli "tendril-source-with-mcp-cli" parentSrc;
        fullSrc = graftMcpCli "tendril-full-source-with-mcp-cli" fullParentSrc;
        # On macOS, linking some dependencies (e.g. anything pulling in
        # `iconv`) needs a libiconv provider. Relying on the ambient `xcrun`
        # SDK is brittle: managed agents sometimes only have a Nix `apple-sdk`
        # on the path that does not ship `libiconv.tbd`, which surfaces as
        # `ld: library not found for -liconv` during direct `cargo` builds and
        # tests (see bd-c88e56). Provide libiconv deterministically through the
        # build/dev environment so validation does not depend on ambient SDK
        # state.
        darwinBuildInputs = lib.optionals pkgs.stdenv.isDarwin [
          pkgs.libiconv
        ];

        commonArgs = {
          inherit src;
          strictDeps = true;
          buildInputs = darwinBuildInputs;
        };
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        commandName = command: command.name or command.exe;
        commandPackageName = command: command.package.pname or command.package.name;
        linkRuntimeCommands = name: commands: pkgs.runCommand name { } (
          ''
            mkdir -p "$out/bin"
          ''
          + lib.concatMapStringsSep "\n"
            (command: ''
              ln -s ${lib.getExe' command.package command.exe} "$out/bin/${commandName command}"
            '')
            commands
        );
        runtimeCommandAudit = name: commands: pkgs.runCommand name { } (
          ''
            mkdir -p "$out"
            cat > "$out/README.md" <<'EOF'
            # Tendril Linux runtime command audit

            This check records the exact executables Tendril exposes through its
            Linux Nix wrappers. Keep this list command-level rather than linking
            whole package `bin/` directories; profile/buildEnv collisions are
            caused by overlapping installed paths, so Tendril should avoid
            contributing incidental tools it never executes.
            EOF
            cat > "$out/commands.tsv" <<'EOF'
            command	package	executable
            EOF
          ''
          + lib.concatMapStringsSep "\n"
            (command: ''
              test -x ${lib.getExe' command.package command.exe}
              printf '%s\t%s\t%s\n' '${commandName command}' '${commandPackageName command}' '${lib.getExe' command.package command.exe}' >> "$out/commands.tsv"
            '')
            commands
          + ''

            duplicates=$(cut -f1 "$out/commands.tsv" | tail -n +2 | sort | uniq -d)
            if [ -n "$duplicates" ]; then
              printf 'duplicate runtime command names:\n%s\n' "$duplicates" >&2
              exit 1
            fi
          ''
        );

        linuxRuntimeCommands = lib.optionals pkgs.stdenv.isLinux [
          # Cross-platform camera capture (V4L2 on Linux)
          { package = pkgs.ffmpeg; exe = "ffmpeg"; }
          # Wayland compositor discovery
          { package = pkgs.hyprland; exe = "hyprctl"; }
          { package = pkgs.sway; exe = "swaymsg"; }
          { package = pkgs.wlr-randr; exe = "wlr-randr"; }
          # Wayland screen capture fallback
          { package = pkgs.grim; exe = "grim"; }
          # Wayland input injection
          { package = pkgs.ydotool; exe = "ydotool"; }
          { package = pkgs.wtype; exe = "wtype"; }
        ];

        linuxHeadlessCommands = lib.optionals pkgs.stdenv.isLinux (
          linuxRuntimeCommands ++ [
            { package = pkgs.bash; exe = "bash"; }
            { package = pkgs.bash; exe = "sh"; }
            { package = pkgs.chromium; exe = "chromium"; }
            { package = pkgs.coreutils; exe = "basename"; }
            { package = pkgs.coreutils; exe = "cat"; }
            { package = pkgs.coreutils; exe = "chmod"; }
            { package = pkgs.coreutils; exe = "dirname"; }
            { package = pkgs.coreutils; exe = "env"; }
            { package = pkgs.coreutils; exe = "mkdir"; }
            { package = pkgs.coreutils; exe = "mktemp"; }
            { package = pkgs.coreutils; exe = "rm"; }
            { package = pkgs.coreutils; exe = "sleep"; }
            { package = pkgs.coreutils; exe = "tail"; }
            { package = pkgs.coreutils; exe = "tr"; }
            { package = pkgs.firefox; exe = "firefox"; }
            { package = pkgs.gnugrep; exe = "grep"; }
            { package = pkgs.openbox; exe = "openbox"; }
            { package = pkgs.python3; exe = "python3"; }
            { package = pkgs.xdpyinfo; exe = "xdpyinfo"; }
            { package = pkgs.xsetroot; exe = "xsetroot"; }
            { package = pkgs.xterm; exe = "xterm"; }
            { package = pkgs.xvfb; exe = "Xvfb"; }
          ]
        );

        linuxRuntimeDeps = lib.unique (map (command: command.package) linuxRuntimeCommands);
        linuxHeadlessDeps = lib.unique (map (command: command.package) linuxHeadlessCommands);
        aarch64LinuxCrossCc = pkgs.pkgsCross.aarch64-multiplatform.stdenv.cc;
        linuxRuntimeBinPath = linkRuntimeCommands "tendril-linux-runtime-bin-path" linuxRuntimeCommands;
        linuxHeadlessBinPath = linkRuntimeCommands "tendril-linux-headless-bin-path" linuxHeadlessCommands;
        linuxRuntimeDependencyAudit = runtimeCommandAudit "tendril-linux-runtime-dependency-audit" linuxHeadlessCommands;

        tendril = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            pname = "tendril";
            version = workspaceVersion;
            cargoExtraArgs = "-p tendril";
            nativeBuildInputs = [ pkgs.makeWrapper ];
            postInstall = ''
              install -Dm755 ${./scripts/tendril-headless.sh} $out/bin/tendril-headless
            '';
            postFixup = lib.optionalString pkgs.stdenv.isLinux ''
              wrapProgram $out/bin/tendril \
                --suffix PATH : ${linuxRuntimeBinPath}/bin
              wrapProgram $out/bin/tendril-headless \
                --prefix PATH : ${linuxHeadlessBinPath}/bin \
                --set-default TENDRIL_HEADLESS_TENDRIL_BIN "$out/bin/tendril"
            '';
            meta = {
              description = "Stateless Rust CLI for agent-driven desktop inspection and control";
              homepage = repositoryUrl;
              license = lib.licenses.mit;
              mainProgram = "tendril";
              platforms = lib.platforms.unix;
            };
          }
        );

        mcpCli = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            pname = "mcp-cli";
            version = workspaceVersion;
            cargoExtraArgs = "-p mcp-cli";
            meta = {
              description = "Reusable JSON envelope and MCP stdio helpers for CLI projects";
              homepage = repositoryUrl;
              license = lib.licenses.mit;
              platforms = lib.platforms.unix;
            };
          }
        );

        clippyCheck = craneLib.cargoClippy (
          commonArgs
          // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--workspace --all-targets --all-features -- -D warnings";
          }
        );

        tests = craneLib.cargoTest (
          commonArgs
          // {
            inherit cargoArtifacts;
            cargoExtraArgs = "--workspace --all-features";
          }
        );

        fmt = pkgs.runCommand "tendril-fmt-check" { nativeBuildInputs = [ pkgs.cargo pkgs.rustfmt ]; } ''
          export HOME="$TMPDIR"
          cp -r ${fullSrc} source
          chmod -R +w source
          cd source
          cargo fmt --all -- --check
          touch "$out"
        '';

        docs = pkgs.runCommand "tendril-docs-check" { nativeBuildInputs = [ pkgs.mdbook ]; } ''
          export HOME="$TMPDIR"
          cp -r ${fullSrc} source
          chmod -R +w source
          cd source
          mdbook build docs
          touch "$out"
        '';
      in
      {
        apps = {
          default = flake-utils.lib.mkApp {
            drv = tendril;
            exePath = "/bin/tendril";
          };
          tendril = flake-utils.lib.mkApp {
            drv = tendril;
            exePath = "/bin/tendril";
          };
          tendril-headless = flake-utils.lib.mkApp {
            drv = tendril;
            exePath = "/bin/tendril-headless";
          };
        };

        packages = {
          default = tendril;
          tendril = tendril;
          mcp-cli = mcpCli;
        } // lib.optionalAttrs pkgs.stdenv.isLinux {
          aarch64-linux-cross-cc = aarch64LinuxCrossCc;
        };

        checks = {
          default = tendril;
          tendril = tendril;
          mcp-cli = mcpCli;
          clippy = clippyCheck;
          inherit tests fmt docs linuxRuntimeDependencyAudit;
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ tendril ];
          packages = with pkgs; [
            cargo
            clippy
            coreutils
            direnv
            ffmpeg
            gnutar
            mdbook
            nixpkgs-fmt
            python3
            rust-analyzer
            rustc
            rustfmt
          ]
          ++ linuxHeadlessDeps
          ++ lib.optionals pkgs.stdenv.isLinux [
            # The aarch64-linux release lane drives Cargo through rustup inside
            # this shell. Its sysroot-aware linker is exposed separately as
            # .#aarch64-linux-cross-cc so native shells keep their host CC.
            pkgs.rustup
          ];
          # Dev-shell environment guards for direct foreground cargo validation.
          #
          # 1. macOS libiconv (bd-c88e56): the Nix apple-sdk does not ship
          #    `libiconv.tbd`, so direct `cargo build`/`test`/`clippy` could
          #    fail with `ld: library not found for -liconv` depending on
          #    whatever SDK `xcrun` resolved. Export LIBRARY_PATH so the linker
          #    always finds the Nix libiconv.
          #
          # 2. clippy/rustc consistency (bd-fbe79c): if an ambient profile
          #    (e.g. ~/.nix-profile or ~/.cargo/bin) puts a `cargo-clippy` /
          #    `clippy-driver` of a different toolchain version ahead of this
          #    shell's matching tools, `cargo clippy` reads rustc artifacts
          #    with a mismatched clippy driver and emits a huge misleading
          #    E0514 "incompatible metadata" cascade. The shell already orders
          #    its matching toolchain first; this guard warns loudly if a
          #    mismatch is still resolvable on PATH so agents are not misled.
          shellHook = ''
            ${lib.optionalString pkgs.stdenv.isDarwin ''
              export LIBRARY_PATH="${pkgs.libiconv}/lib''${LIBRARY_PATH:+:$LIBRARY_PATH}"
            ''}
            __tendril_rustc_ver="$(rustc -vV 2>/dev/null | awk -F': ' '/^release/{print $2}')"
            __tendril_clippy_ver="$(clippy-driver -vV 2>/dev/null | awk -F': ' '/^release/{print $2}')"
            if [ -n "$__tendril_rustc_ver" ] && [ -n "$__tendril_clippy_ver" ] \
              && [ "$__tendril_rustc_ver" != "$__tendril_clippy_ver" ]; then
              echo "tendril dev shell warning (bd-fbe79c): rustc $__tendril_rustc_ver != clippy-driver $__tendril_clippy_ver on PATH." >&2
              echo "  Direct 'cargo clippy' may emit a misleading E0514 cascade. Run clippy via" >&2
              echo "  the flake check instead: nix build .#checks.\$(nix eval --raw --impure --expr builtins.currentSystem).clippy" >&2
              echo "  rustc:         $(command -v rustc)" >&2
              echo "  clippy-driver: $(command -v clippy-driver)" >&2
            fi
            unset __tendril_rustc_ver __tendril_clippy_ver
          '';
        };

        formatter = pkgs.nixpkgs-fmt;
      }
    );
}
