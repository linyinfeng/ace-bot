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
    useDHCP = false; # disable deprecated option
    useNetworkd = true;
    interfaces.enp1s0.useDHCP = true;
    firewall.enable = true;
  };
  services.openssh.enable = true;
  services.fail2ban.enable = true;

  environment.defaultPackages = lib.mkForce [ ];

  users.users.root = {
    openssh.authorizedKeys.keyFiles = [
      ../public/id_ed25519.pub
    ];
  };

  environment.persistence."/persist" = {
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
    preStart = ''
      echo "setup ace-bot token"
      cp "${config.sops.secrets.ace-bot.path}" "$RUNTIME_DIRECTORY/token"
      chown ace-bot:root "$RUNTIME_DIRECTORY/token"
      chmod 400 "$RUNTIME_DIRECTORY/token"
    '';
    script = ''
      cd $RUNTIME_DIRECTORY
      echo "read ace-bot token"
      export TELOXIDE_TOKEN=$(cat token)
      echo "clear ace-bot token"
      rm token
      ${pkgs.ace-bot}/bin/ace-bot
    '';

    serviceConfig = {
      DynamicUser = true;
      PermissionsStartOnly = true;
      Restart = "always";
      LimitNPROC = "100";
      RuntimeDirectory = "ace-bot";
      SupplementaryGroups = [ "nix-allowed" ];
    };

    path = with pkgs; [
      bash
      mount
      coreutils
      procps
      curl
      netcat.nc
      vim
      which
    ];

    environment = {
      "MANAGER_CHAT_ID" = "148111617";
      "RUST_LOG" = "info";
    };

    wantedBy = [ "multi-user.target" ];
  };
  sops.secrets.ace-bot = { };
  users.groups.nix-allowed = { };

  security.auditd.enable = true;
  security.audit.enable = true;
  security.audit.rules = [
    # log all commands executed
    "-a exit,always -F arch=b64 -S execve"
  ];

  nix.settings.trusted-users = [ "root" "@wheel" ];
  nix.settings.allowed-users = [ "@nix-allowed" ];

  fileSystems."/" =
    {
      device = "tmpfs";
      fsType = "tmpfs";
      options = [ "defaults" "size=2G" "mode=755" "noexec" ];
    };
  fileSystems."/persist" = btrfsSubvolMain "@persist" { neededForBoot = true; options = [ "noexec" ]; };
  fileSystems."/var/log" = btrfsSubvolMain "@var-log" { neededForBoot = true; options = [ "noexec" ]; };
  fileSystems."/nix" = btrfsSubvolMain "@nix" { neededForBoot = true; };
  fileSystems."/swap" = btrfsSubvolMain "@swap" { options = [ "noexec" ]; };
  fileSystems."/boot" =
    {
      device = "/dev/disk/by-uuid/5C56-7693";
      fsType = "vfat";
      options = [ "noexec" ];
    };
  swapDevices =
    [{
      device = "/swap/swapfile";
    }];

  system.stateVersion = "22.05";
}
