{ config, pkgs, lib, modulesPath, ... }:

let
  cfg = config.services.ace-bot;
in
{
  options.services.ace-bot = {
    enable = lib.mkEnableOption "ace-bot";
    managedChatId = lib.mkOption {
      type = lib.types.str;
    };
    logLevel = lib.mkOption {
      type = lib.types.str;
      default = "info";
    };
    acePackages = lib.mkOption {
      type = with list.types; listOf package;
    };
  };

  config = lib.mkIf cfg.enable {
    services.ace-bot.acePackages = with pkgs; [
      bash
    ];

    systemd.services.ace-bot = {
      script = ''
        export TELOXIDE_TOKEN=$(cat "$CREDENTIALS_DIRECTORY/token")
        rm "$CREDENTIALS_DIRECTORY/token"
        cd $RUNTIME_DIRECTORY
        ${pkgs.ace-bot}/bin/ace-bot
      '';

      serviceConfig = {
        DynamicUser = true;
        LoadCredential = [
          "token:${config.sops.secrets.ace-bot.path}"
        ];
        Restart = "always";
        RuntimeDirectory = "ace-bot";
        LimitNPROC = 100;
      };

      path = with pkgs; [
        bash
      ];

      environment = {
        "MANAGER_CHAT_ID" = cfg.managedChatId;
        "RUST_LOG" = cfg.logLevel;
      };

      wantedBy = [ "multi-user.target" ];
    };
  }
}
