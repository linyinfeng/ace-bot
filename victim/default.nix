{
  config,
  pkgs,
  lib,
  modulesPath,
  ...
}: {
  boot.isContainer = true;

  networking.useDHCP = false;

  systemd.services.ace-bot = {
    preStart = ''
      echo "setup ace-bot token"
      cp /secrets/ace-bot-token "$RUNTIME_DIRECTORY/token"
      chown ace-bot:root "$RUNTIME_DIRECTORY/token"
      chmod 400 "$RUNTIME_DIRECTORY/token"
    '';
    script = ''
      echo "read ace-bot token"
      export TELOXIDE_TOKEN=$(cat "$RUNTIME_DIRECTORY/token")
      echo "clear ace-bot token"
      rm "$RUNTIME_DIRECTORY/token"

      export HOME="$PWD"
      ${pkgs.ace-bot}/bin/ace-bot
    '';

    serviceConfig = {
      DynamicUser = true;
      SupplementaryGroups = ["nix-allowed"];
      PermissionsStartOnly = true;

      StateDirectory = "ace-bot";
      RuntimeDirectory = "ace-bot";
      WorkingDirectory = "/var/lib/ace-bot";

      LoadCredential = [
        "token:/secrets/ace-bot-token"
      ];

      Restart = "always";
      LimitNPROC = "100";
    };

    path = with pkgs; [
      bash
      "/run/wrappers"
      "/run/current-system/sw"
    ];

    environment = {
      "MANAGER_CHAT_ID" = "148111617";
      "RUST_LOG" = "info";
    };

    wantedBy = ["multi-user.target"];
  };
  users.groups.nix-allowed = {};
  nix.settings.allowed-users = ["@nix-allowed"];

  system.stateVersion = "22.11";
}
