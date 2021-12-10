{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  packages = with pkgs; [
    rustup
    rust-analyzer

    sops
    ssh-to-age
  ];

  RUST_LOG = "info,commit-notifier=debug";
}
