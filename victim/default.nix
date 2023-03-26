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
    script = ''
      echo "read ace-bot token"
      export TELOXIDE_TOKEN=$(cat "$CREDENTIALS_DIRECTORY/token")
      echo "clear ace-bot token"
      rm "$CREDENTIALS_DIRECTORY/token"

      ${pkgs.ace-bot}/bin/ace-bot
    '';

    serviceConfig = {
      DynamicUser = true;
      SupplementaryGroups = ["nix-allowed"];
      PermissionsStartOnly = true;

      StateDirectory = "ace-bot";
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
