{
  nixConfig.extra-experimental-features = "nix-command flakes ca-references";
  nixConfig.extra-substituters = "https://linyinfeng.cachix.org https://nix-community.cachix.org";
  nixConfig.extra-trusted-public-keys = "linyinfeng.cachix.org-1:sPYQXcNrnCf7Vr7T0YmjXz5dMZ7aOKG3EqLja0xr9MM= nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs=";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils-plus.url = "github:gytis-ivaskevicius/flake-utils-plus";
  };
  outputs =
    inputs@{ self, nixpkgs, flake-utils-plus, impermanence, sops-nix }:
    let utils = flake-utils-plus.lib;
    in
    utils.mkFlake {
      inherit self inputs;

      sharedOverlays = [
        (final: prev: {
          ace-bot = final.callPackage ./bot.nix { };
        })
      ];

      nixosModules.ace-bot = ./modules/services/ace-bot.nix;

      outputsBuilder = channels:
        let pkgs = channels.nixpkgs;
        in
        {
          packages.ace-bot = pkgs.callPackage ./bot.nix { };
          devShell = pkgs.callPackage ./shell.nix { };
        };
    };
}
