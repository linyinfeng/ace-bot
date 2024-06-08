{
  config,
  pkgs,
  lib,
  ...
}: let
  cfg = config.services.ace-bot;
in {
  options.services.ace-bot = {
    enable = lib.mkEnableOption "ace-bot";
    managerChatId = lib.mkOption {
      type = with lib.types; nullOr str;
      default = null;
    };
    tokenFile = lib.mkOption {
      type = lib.types.path;
    };
  };
  config = lib.mkIf (cfg.enable) {
    users.users.ace-bot = {
      isNormalUser = true;
      group = "ace-bot";
      home = "/var/lib/ace-bot";
      shell = pkgs.bashInteractive;
      linger = true;
    };
    systemd.tmpfiles.settings."80-ace-bot" = let
      ownerOptions = {
        user = config.users.users.ace-bot.name;
        group = config.users.users.ace-bot.group;
      };
    in {
      ${config.users.users.ace-bot.home} = {
        "d" = {
          mode = "0700";
          inherit (ownerOptions) user group;
        };
        "Z" = {
          mode = "0700";
          inherit (ownerOptions) user group;
        };
      };
    };
    users.groups.ace-bot = {};
    systemd.services.ace-bot = {
      script = ''
        export TELOXIDE_TOKEN=$(cat "$CREDENTIALS_DIRECTORY/token")
        exec ${pkgs.ace-bot}/bin/ace-bot
      '';

      serviceConfig = {
        LoadCredential = [
          "token:${cfg.tokenFile}"
        ];
        Restart = "always";
      };

      environment =
        {
          "RUST_LOG" = "info";
          "SHELL" = "${lib.getExe config.users.users.ace-bot.shell}";
        }
        // lib.optionalAttrs (cfg.managerChatId != null) {
          "MANAGER_CHAT_ID" = cfg.managerChatId;
        };

      wantedBy = ["multi-user.target"];
    };
  };
}
