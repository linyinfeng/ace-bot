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
      linger = true;
    };
    systemd.tmpfiles.settings."80-ace-bot" =
    let
      ownerOptions = {
        user = config.users.users.ace-bot.name;
        group = config.users.users.ace-bot.group;
      };
    in
    {
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
        echo "read ace-bot token"
        export TELOXIDE_TOKEN=$(cat "$RUNTIME_DIRECTORY/token")
        echo "clear ace-bot token"
        rm "$RUNTIME_DIRECTORY/token"

        export XDG_RUNTIME_DIR="/run/user/$UID"
        export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
        exec ${pkgs.ace-bot}/bin/ace-bot
      '';

      serviceConfig = {
        ExecStartPre = let
          setupCredential = pkgs.writeShellScript "ace-bot-setup-credential" ''
            echo "setup ace-bot token"
            cp "${cfg.tokenFile}" "$RUNTIME_DIRECTORY/token"
            chown ace-bot:ace-bot "$RUNTIME_DIRECTORY/token"
            chmod 400 "$RUNTIME_DIRECTORY/token"
          '';
        in "+${setupCredential}";
        User = "ace-bot";
        Group = "ace-bot";

        StateDirectory = "ace-bot";
        RuntimeDirectory = "ace-bot";
        WorkingDirectory = "/var/lib/ace-bot";

        Restart = "always";
      };

      path = with pkgs; [
        bash
      ];

      environment =
        {
          "RUST_LOG" = "info";
          "HOME" = "/var/lib/ace-bot";
        }
        // lib.optionalAttrs (cfg.managerChatId != null) {
          "MANAGER_CHAT_ID" = cfg.managerChatId;
        };

      wantedBy = ["multi-user.target"];
    };
  };
}
