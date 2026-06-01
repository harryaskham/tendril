{
  description = "Tendril Rust workspace and Nix development environment";

  inputs = {
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    mcp-cli = {
      url = "github:harryaskham/mcp-cli/9e2f1fc3fe71cd757cea3cbd4943b2b60525a548";
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
        releaseTag = "v${workspaceVersion}";
        releaseTarget =
          if system == "x86_64-linux" then
            "x86_64-linux"
          else if system == "aarch64-linux" then
            "aarch64-linux"
          else if system == "aarch64-darwin" then
            "aarch64-darwin"
          else if system == "x86_64-darwin" then
            "x86_64-darwin"
          else
            throw "unsupported Tendril release target for system `${system}`";
        releaseArtifactName = "tendril-${workspaceVersion}-${releaseTarget}.tar.gz";
        releaseChecksumName = "tendril-${workspaceVersion}-${releaseTarget}.sha256";
        repositoryUrl = "https://github.com/harryaskham/tendril";
        releaseManifest = pkgs.writeText "tendril-release-manifest.json" (
          builtins.toJSON {
            project = "tendril";
            version = workspaceVersion;
            semver = workspaceVersion;
            tag = releaseTag;
            trigger = "tag_push";
            system = system;
            platform = releaseTarget;
            nix = {
              package = "tendril";
              release_package = "releaseArtifact";
            };
            artifacts = [
              {
                name = releaseArtifactName;
                kind = "archive";
                format = "tar.gz";
              }
              {
                name = releaseChecksumName;
                kind = "checksum";
                format = "sha256";
              }
            ];
            repository = repositoryUrl;
          }
        );
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

        releaseArtifact = pkgs.runCommand "tendril-release-${releaseTarget}"
          {
            nativeBuildInputs = [ pkgs.coreutils pkgs.gnutar pkgs.gzip ];
          } ''
          mkdir -p "$out" "stage/tendril-${workspaceVersion}-${releaseTarget}"
          cp ${tendril}/bin/tendril "stage/tendril-${workspaceVersion}-${releaseTarget}/tendril"
          cp ${tendril}/bin/tendril-headless "stage/tendril-${workspaceVersion}-${releaseTarget}/tendril-headless"
          chmod +x "stage/tendril-${workspaceVersion}-${releaseTarget}/tendril" "stage/tendril-${workspaceVersion}-${releaseTarget}/tendril-headless"
          tar \
            --sort=name \
            --mtime='UTC 1970-01-01' \
            --owner=0 \
            --group=0 \
            --numeric-owner \
            --use-compress-program="gzip -n" \
            -cf "$out/${releaseArtifactName}" \
            -C stage \
            "tendril-${workspaceVersion}-${releaseTarget}"
          (
            cd "$out"
            sha256sum "${releaseArtifactName}" > "${releaseChecksumName}"
          )
          install -m644 ${releaseManifest} "$out/release-manifest.json"
        '';

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
          releaseArtifact = releaseArtifact;
        };

        checks = {
          default = tendril;
          tendril = tendril;
          mcp-cli = mcpCli;
          releaseArtifact = releaseArtifact;
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
            gnutar
            mdbook
            nixpkgs-fmt
            rust-analyzer
            rustc
            rustfmt
          ] ++ linuxHeadlessDeps;
          # Ensure direct foreground `cargo build`/`cargo test`/`cargo clippy`
          # from this dev shell can link `-liconv` on macOS without depending on
          # whatever SDK `xcrun` happens to resolve (bd-c88e56). We prepend the
          # Nix libiconv lib dir to LIBRARY_PATH so the linker always finds it.
          shellHook = lib.optionalString pkgs.stdenv.isDarwin ''
            export LIBRARY_PATH="${pkgs.libiconv}/lib''${LIBRARY_PATH:+:$LIBRARY_PATH}"
          '';
        };

        formatter = pkgs.nixpkgs-fmt;
      }
    );
}
