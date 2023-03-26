{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";

    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    flake-utils.url = "github:numtide/flake-utils";

    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";

    crane.url = "github:ipetkov/crane";
    crane.inputs.nixpkgs.follows = "nixpkgs";
    crane.inputs.flake-compat.follows = "flake-compat";
    crane.inputs.flake-utils.follows = "flake-utils";
    crane.inputs.rust-overlay.follows = "rust-overlay";

    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.flake-utils.follows = "flake-utils";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";

    flake-compat.url = "github:edolstra/flake-compat";
    flake-compat.flake = false;
  };

  outputs = inputs @ {flake-parts, ...}:
    flake-parts.lib.mkFlake {inherit inputs;}
    ({
      config,
      self,
      inputs,
      lib,
      getSystem,
      ...
    }: {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      imports = [
        inputs.flake-parts.flakeModules.easyOverlay
        inputs.treefmt-nix.flakeModule
      ];
      flake = let
        mkHostAllSystems = {
          name,
          modules,
        }:
          lib.mkMerge (
            lib.lists.map
            (system: {
              "${name}-${system}" = lib.nixosSystem {
                inherit system;
                modules = modules ++ [{nixpkgs.overlays = [config.flake.overlays.default];}];
              };
            })
            config.systems
          );
      in {
        nixosModules.victim = ./victim;
        nixosConfigurations = mkHostAllSystems {
          name = "victim";
          modules = [
            config.flake.nixosModules.victim
          ];
        };
      };
      perSystem = {
        config,
        self',
        pkgs,
        system,
        ...
      }: let
        craneLib = inputs.crane.lib.${system};
        src = craneLib.cleanCargoSource ./.;
        bareCommonArgs = {
          inherit src;
          nativeBuildInputs = with pkgs; [
            pkg-config
          ];
          buildInputs = with pkgs; [
            openssl
            sqlite
          ];
        };
        cargoArtifacts = craneLib.buildDepsOnly bareCommonArgs;
        commonArgs = bareCommonArgs // {inherit cargoArtifacts;};
      in {
        packages = {
          ace-bot = craneLib.buildPackage commonArgs;
          default = config.packages.ace-bot;
        };
        overlayAttrs = {
          inherit (config.packages) ace-bot;
        };
        checks = {
          inherit (self'.packages) ace-bot;
          doc = craneLib.cargoDoc commonArgs;
          fmt = craneLib.cargoFmt {inherit src;};
          nextest = craneLib.cargoNextest commonArgs;
          clippy = craneLib.cargoClippy (commonArgs
            // {
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            });
        };
        treefmt = {
          projectRootFile = "flake.nix";
          programs = {
            alejandra.enable = true;
            rustfmt.enable = true;
            shfmt.enable = true;
          };
        };
        devShells.default = pkgs.mkShell {
          inputsFrom = lib.attrValues self'.checks;
          packages = with pkgs; [
            rustup
            rust-analyzer
          ];
        };
      };
    });
}
