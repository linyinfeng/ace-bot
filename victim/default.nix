{ config, pkgs, lib, modulesPath, ... }:

let
  btrfsSubvol = device: subvol: extraConfig: lib.mkMerge [
    {
      inherit device;
      fsType = "btrfs";
      options = [ "subvol=${subvol}" "compress=zstd" ];
    }
    extraConfig
  ];
  btrfsSubvolMain = btrfsSubvol "/dev/disk/by-uuid/9f227a19-d570-449f-b4cb-0eecc5b2d227";
in
{
  imports = [
    (import "${modulesPath}/profiles/hardened.nix")
  ];

  i18n.defaultLocale = "en_US.UTF-8";
  console.keyMap = "us";
  time.timeZone = "Asia/Shanghai";

  boot.loader.grub = {
    enable = true;
    version = 2;
    device = "/dev/vda";
  };
  boot.initrd.availableKernelModules = [ "ata_piix" "uhci_hcd" "virtio_pci" "sr_mod" "virtio_blk" ];

  networking = {
    useNetworkd = true;
    interfaces.ens3.useDHCP = true;
    firewall.enable = true;
  };
  services.openssh.enable = true;
  services.fail2ban.enable = true;

  users.users.root = {
    openssh.authorizedKeys.keyFiles = [
      ../public/id_ed25519.pub
    ];
  };

  environment.persistence."/nix/persist" = {
    directories = [
    ];
    files = [
      "/etc/machine-id"
      "/etc/ssh/ssh_host_rsa_key"
      "/etc/ssh/ssh_host_rsa_key.pub"
      "/etc/ssh/ssh_host_ed25519_key"
      "/etc/ssh/ssh_host_ed25519_key.pub"
    ];
  };
  sops.defaultSopsFile = lib.mkDefault ../secrets/main.yaml;
  sops.gnupg.sshKeyPaths = [ ];
  sops.age.sshKeyPaths = lib.mkDefault [
    "/persist/etc/ssh/ssh_host_ed25519_key"
  ];

  systemd.services.ace-bot = {
    enable = false;
    script = ''
      export TELOXIDE_TOKEN=$(cat "$CREDENTIALS_DIRECTORY/token")
      ${pkgs.ace-bot}/bin/ace-bot
    '';

    serviceConfig = {
      DynamicUser = true;
      LoadCredential = [
        "token:${config.sops.secrets.ace-bot.path}"
      ];
      Restart = "always";
    };

    environment."RUST_LOG" = "info";

    wantedBy = [ "multi-user.target" ];
  };
  sops.secrets.ace-bot = { };

  fileSystems."/" =
    {
      device = "tmpfs";
      fsType = "tmpfs";
      options = [ "defaults" "size=2G" "mode=755" ];
    };
  fileSystems."/persist" = btrfsSubvolMain "@persist" { neededForBoot = true; };
  fileSystems."/var/log" = btrfsSubvolMain "@var-log" { neededForBoot = true; };
  fileSystems."/nix" = btrfsSubvolMain "@nix" { neededForBoot = true; };
  fileSystems."/swap" = btrfsSubvolMain "@swap" { };
  fileSystems."/boot" =
    {
      device = "/dev/disk/by-uuid/4a186796-5865-4b47-985c-9354adec09a4";
      fsType = "ext4";
    };
  swapDevices =
    [{
      device = "/swap/swapfile";
    }];
}
