{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils-plus.url = "github:gytis-ivaskevicius/flake-utils-plus";
    impermanence.url = "github:nix-community/impermanence";
    sops-nix.url = "github:mic92/sops-nix";
    sops-nix.inputs.nixpkgs.follows = "nixpkgs";
    deploy-rs.url = "github:serokell/deploy-rs";
    deploy-rs.inputs.nixpkgs.follows = "nixpkgs";
    deploy-rs.inputs.utils.follows = "flake-utils-plus/flake-utils";
  };
  outputs =
    inputs@{ self, nixpkgs, flake-utils-plus, impermanence, sops-nix, deploy-rs }:
    let utils = flake-utils-plus.lib;
    in
    utils.mkFlake {
      inherit self inputs;

      sharedOverlays = [
        (final: prev: {
          ace-bot = final.callPackage ./bot.nix { };
        })
      ];
      hostDefaults.channelName = "nixpkgs";

      hosts.victim = {
        system = "x86_64-linux";
        modules = [
          ./victim
          impermanence.nixosModules.impermanence
          sops-nix.nixosModules.sops
        ];
      };

      deploy.nodes.victim.profiles.system = {
        user = "root";
        path = deploy-rs.lib.x86_64-linux.activate.nixos
          self.nixosConfigurations.victim;
      };

      outputsBuilder = channels:
        let pkgs = channels.nixpkgs;
        in
        {
          packages.bot = pkgs.callPackage ./bot.nix { };
          devShell = pkgs.callPackage ./shell.nix { };
        };
    };
}
