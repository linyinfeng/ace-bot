{
  config,
  pkgs,
  lib,
  ...
}: let
  cfg = config.services.ace-bot;
  hostConfig = config;
  nixosSystem = args:
    import "${pkgs.path}/nixos/lib/eval-config.nix" ({
        inherit lib;
        system = null;
        modules =
          args.modules
          ++ [
            ({modulesPath, ...}: {
              imports = [
                "${modulesPath}/misc/nixpkgs/read-only.nix"
              ];
              nixpkgs.pkgs = pkgs;
              system = {inherit (config.system) stateVersion;};
            })
          ];
      }
      // removeAttrs args ["modules"]);
  envConfiguration = nixosSystem {
    modules =
      [
        ({modulesPath, ...}: {
          # container settings
          boot.isContainer = true;
          networking.useNetworkd = true;
          networking.useDHCP = false;
          networking.useHostResolvConf = false;
          services.resolved.enable = true;
        })
        ({
          config,
          pkgs,
          ...
        }: {
          environment.systemPackages = [cfg.shell];
          nix.package = lib.mkDefault hostConfig.nix.package;
          systemd.services."setup-nix-db" = {
            script = ''
              if [ ! -d /nix/var/nix/db ]; then
                nix-store --load-db </nix/initial-registration
              fi
            '';
            path = [
              config.nix.package
            ];
            serviceConfig = {
              Type = "oneshot";
            };
            before = ["nix-daemon.service"];
            wantedBy = ["multi-user.target"];
          };
        })
        ({pkgs, ...}: {
          # extra nix settings
          nix.settings = {
            allowed-users = ["ace-bot"];
            use-xdg-base-directories = true;
            experimental-features = [
              "nix-command"
              "flakes"
            ];
          };
        })
      ]
      ++ cfg.extraModules;
  };
  envToplevel = envConfiguration.config.system.build.toplevel;
  envToplevelClosureInfo = pkgs.closureInfo {rootPaths = [envToplevel];};
  envToplevelState =
    pkgs.runCommand "toplevel-state" {
      buildInputs = [
        envToplevel
      ];
      nativeBuildInputs = with pkgs; [
        nix
      ];
    } ''
      mkdir -p "$out"
      export NIX_STATE_DIR="$out"
      nix-store --load-db <"${envToplevelClosureInfo}/registration"
    '';
  envInitName =
    if envConfiguration.config.boot.initrd.systemd.enable
    then "prepare-root"
    else "init";
  envInit = "${envToplevel}/${envInitName}";
  nspawnSettingsBase = pkgs.writeText "ace-bot.nspawn.base" ''
    [Exec]
    Boot=no
    Parameters="${envInit}"
    PrivateUsers=${toString cfg.privateUsers.uidBase}:${toString cfg.privateUsers.gidBase}
    LinkJournal=host

    [Files]
    PrivateUsersOwnership=map
    BindUser=ace-bot
    BindReadOnly=${envToplevelClosureInfo}/registration:/nix/initial-registration:idmap

    [Network]
  '';
  nspawnSettings = pkgs.runCommand "ace-bot.nspawn" {} ''
    touch "$out"
    cp "${nspawnSettingsBase}" "$out"
    echo "[Files]" >>"$out"

    IFS=$'\n'
    for store_path in $(cat "${envToplevelClosureInfo}/store-paths"); do
      echo "BindReadOnly=$store_path:$store_path:idmap" >>"$out"
    done
  '';
in {
  options.services.ace-bot = {
    enable = lib.mkEnableOption "ace-bot";
    extraModules = lib.mkOption {
      type = with lib.types; listOf unspecified;
      default = [];
    };
    containerConfig = lib.mkOption {
      type = with lib.types; unspecified;
      readOnly = true;
      default = envConfiguration;
    };
    disk = {
      size = lib.mkOption {
        type = with lib.types; nullOr str;
        default = "5GiB";
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
    privateUsers = {
      uidBase = lib.mkOption {
        type = with lib.types;
          int
          // {
            check = v: int.check v && 524288 <= v && v <= 1878982656;
          };
        default = 7077888;
      };
      gidBase = lib.mkOption {
        type = with lib.types;
          int
          // {
            check = v: int.check v && 524288 <= v && v <= 1878982656;
          };
        default = 7077888;
      };
    };
  };
  config = lib.mkIf (cfg.enable) {
    users.users.ace-bot = {
      isSystemUser = true;
      home = "/var/lib/ace-bot/mount/disk/home";
      shell = cfg.shell;
      group = "ace-bot";
    };
    users.groups.ace-bot = {};
    systemd.services.ace-bot = {
      script = ''
        # setup token
        export TELOXIDE_TOKEN=$(cat "$CREDENTIALS_DIRECTORY/token")
        exec ${pkgs.ace-bot}/bin/ace-bot \
          --shell="${lib.getExe cfg.shell}" \
          --timeout="${cfg.timeout}" \
          ${lib.optionalString (cfg.managerChatId != null) ''--manager-chat-id="${cfg.managerChatId}"''} \
          --machine="ace-bot" \
          --reset-indicator="/var/lib/machines/ace-bot/reset" \
          --machine-unit="systemd-nspawn@ace-bot.service" \
          --user-mode-uid="ace-bot" \
          --user-mode-gid="ace-bot" \
          --user-home="/run/host/home/ace-bot" \
          ${lib.escapeShellArgs cfg.extraOptions}
      '';
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
        CPUWeight = lib.mkDefault "idle";
        CPUQuota = lib.mkDefault "50%";
        MemoryMax = lib.mkDefault "1G";
        MemorySwapMax = lib.mkDefault "2G";
        LimitNProc = lib.mkDefault 10240;
      };
    };
    systemd.targets.machines.wants = ["systemd-nspawn@ace-bot.service"];
    systemd.services."systemd-nspawn@ace-bot" = {
      overrideStrategy = "asDropin";
      serviceConfig = {
        Restart = "always";
        Slice = "acebot.slice";
      };
      restartTriggers = [
        nspawnSettings
      ];
    };
    systemd.services."ace-bot-image" = {
      wantedBy = ["systemd-nspawn@ace-bot.service"];
      before = ["systemd-nspawn@ace-bot.service"];
      partOf = ["systemd-nspawn@ace-bot.service"];
      script = ''
        set -x

        if [ ! -L toplevel ] ||
           [ "${envToplevel}" != "$(readlink toplevel)" ]; then
          clean_store=1
        else
          clean_store=0
        fi
        rm toplevel
        nix --experimental-features nix-command build "${envToplevel}" --out-link toplevel

        # setup disk
        if [ ! -f disk ]; then
          fallocate --length "${cfg.disk.size}" disk
          mkfs.ext4 disk
        fi
        mkdir --parents mount/disk
        mount disk mount/disk
        mkdir --parents mount/disk/home
        mkdir --parents mount/disk/root
        chown --recursive ace-bot:ace-bot mount/disk/home

        # clean up store
        if [ "$clean_store" = "1" ]; then
          rm --recursive --force mount/disk/root/nix
        fi

        # setup image root
        mkdir --parents /var/lib/machines/ace-bot
        mount --bind mount/disk/root /var/lib/machines/ace-bot
      '';
      postStop = ''
        set +e
        set -x
        umount mount/disk
        rmdir mount/disk
        rmdir mount
        if [ -f /var/lib/machines/ace-bot/reset ]; then
          rm disk
        fi
        umount /var/lib/machines/ace-bot
        rm --recursive --force /var/lib/machines/ace-bot
      '';
      path =
        [config.nix.package]
        ++ (with pkgs; [
          util-linux
          e2fsprogs
        ]);
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        StateDirectory = "ace-bot";
        WorkingDirectory = "/var/lib/ace-bot";
      };
    };
    environment.etc."systemd/nspawn/ace-bot.nspawn".source = nspawnSettings;
    networking.firewall.allowedUDPPorts = [
      67 # DHCP server
    ];
    passthru = {
      aceBot = {
        inherit envToplevelState;
        inherit envToplevelClosureInfo;
        inherit nspawnSettings;
      };
    };
  };
}
