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
        commonArgs = {
          inherit src;
          strictDeps = true;
        };
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        linuxRuntimeDeps = lib.optionals pkgs.stdenv.isLinux (with pkgs; [
          # Wayland compositor discovery
          hyprland # hyprctl
          sway # swaymsg
          wlr-randr
          # Wayland screen capture fallback
          grim
          # Wayland input injection
          ydotool
          wtype
        ]);

        linuxHeadlessDeps = lib.optionals pkgs.stdenv.isLinux (with pkgs; [
          bash
          chromium
          coreutils
          firefox
          openbox
          python3
          xdpyinfo
          xsetroot
          xterm
          xvfb
        ]);

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
                --suffix PATH : ${lib.makeBinPath linuxRuntimeDeps}
              wrapProgram $out/bin/tendril-headless \
                --prefix PATH : ${lib.makeBinPath (linuxRuntimeDeps ++ linuxHeadlessDeps)} \
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
          mkdir -p "$out" stage
          cp ${tendril}/bin/tendril stage/tendril
          cp ${tendril}/bin/tendril-headless stage/tendril-headless
          chmod +x stage/tendril stage/tendril-headless
          tar \
            --sort=name \
            --mtime='UTC 1970-01-01' \
            --owner=0 \
            --group=0 \
            --numeric-owner \
            --use-compress-program="gzip -n" \
            -cf "$out/${releaseArtifactName}" \
            -C stage \
            tendril \
            tendril-headless
          (
            cd "$out"
            sha256sum "${releaseArtifactName}" > "${releaseChecksumName}"
          )
          install -m644 ${releaseManifest} "$out/release-manifest.json"
        '';

        clippy = craneLib.cargoClippy (
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
          inherit clippy tests fmt docs;
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
        };

        formatter = pkgs.nixpkgs-fmt;
      }
    );
}
