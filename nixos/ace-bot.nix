{
  config,
  pkgs,
  lib,
  ...
}: let
  cfg = config.services.ace-bot;
  env = pkgs.buildEnv {
    name = "ace-bot-env";
    paths = cfg.packages;
  };
in {
  options.services.ace-bot = {
    enable = lib.mkEnableOption "ace-bot";
    packages = lib.mkOption {
      type = with lib.types; listOf package;
      default = with pkgs; [ coreutils ];
    };
    disk = {
      size = lib.mkOption {
        type = with lib.types; nullOr str;
        default = "1GiB";
      };
    };
    managerChatId = lib.mkOption {
      type = with lib.types; nullOr str;
      default = null;
    };
    timeout = lib.mkOption {
      type = with lib.types; nullOr str;
      default = "60";
    };
    shell = lib.mkOption {
      type = with lib.types; package;
      default = pkgs.bashInteractive;
      defaultText = "pkgs.bashInteractive";
    };
    extraOptions = lib.mkOption {
      type = with lib.types; listOf str;
      default = [];
    };
    tokenFile = lib.mkOption {
      type = lib.types.path;
    };
    rustLog = lib.mkOption {
      type = lib.types.str;
      default = "info";
    };
  };
  config = lib.mkIf (cfg.enable) {
    users.users.ace-bot = {
      isSystemUser = true;
      group = "ace-bot";
      home = "/var/lib/ace-bot/home";
      shell = cfg.shell;
    };
    users.groups.ace-bot = { };
    systemd.services.ace-bot = {
      script = ''
        # setup token
        export TELOXIDE_TOKEN=$(cat "$CREDENTIALS_DIRECTORY/token")

        # setup disk
        if [ ! -f disk ]; then
          fallocate -l "${cfg.disk.size}" disk
          mkfs.ext4 disk
        fi
        mkdir --parents home
        mount disk home
        chown --recursive ace-bot:ace-bot home

        # setup root
        rm --recursive --force root
        mkdir --parents root

        exec ${pkgs.ace-bot}/bin/ace-bot \
          --shell="${lib.getExe cfg.shell}" \
          --timeout="${cfg.timeout}" \
          ${lib.optionalString (cfg.managerChatId != null) ''--manager-chat-id="${cfg.managerChatId}"''} \
          --working-directory="/var/lib/ace-bot/home" \
          --root-directory="/var/lib/ace-bot/root" \
          --environment="${env}"
          ${lib.escapeShellArgs cfg.extraOptions}
      '';
      postStop = ''
        umount home
      '';
      path = with pkgs; [
        util-linux
        e2fsprogs
      ];
      serviceConfig = {
        LoadCredential = [
          "token:${cfg.tokenFile}"
        ];
        StateDirectory = "ace-bot";
        WorkingDirectory = "/var/lib/ace-bot";
        Restart = "always";
      };
      environment = {
        "RUST_LOG" = cfg.rustLog;
      };
      wantedBy = ["multi-user.target"];
    };
    systemd.slices.acebot = {
      description = "ACE Bot Remote Codes";
      sliceConfig = {
        CPUWeight = "idle";
        CPUQuota = "50%";
        MemoryMax = "128M";
        MemorySwapMax = "512M";
        LimitNPROC = "100";
      };
    };
  };
}
