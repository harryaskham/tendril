{
  description = "Tendril Rust workspace and Nix development environment";

  inputs = {
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = {
    self,
    crane,
    flake-utils,
    nixpkgs,
  }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        craneLib = crane.mkLib pkgs;
        src = craneLib.cleanCargoSource ./.;
        commonArgs = {
          inherit src;
          strictDeps = true;
        };
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        tendril = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            pname = "tendril";
            cargoExtraArgs = "-p tendril";
          }
        );

        mcpCli = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            pname = "mcp-cli";
            cargoExtraArgs = "-p mcp-cli";
          }
        );

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
          cp -r ${./.} source
          chmod -R +w source
          cd source
          cargo fmt --all -- --check
          touch "$out"
        '';
      in
      {
        packages = {
          default = tendril;
          tendril = tendril;
          mcp-cli = mcpCli;
        };

        checks = {
          default = tendril;
          tendril = tendril;
          mcp-cli = mcpCli;
          inherit clippy tests fmt;
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ tendril ];
          packages = with pkgs; [
            cargo
            clippy
            direnv
            nixpkgs-fmt
            rust-analyzer
            rustc
            rustfmt
          ];
        };

        formatter = pkgs.nixpkgs-fmt;
      }
    );
}
