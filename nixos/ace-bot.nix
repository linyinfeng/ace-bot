{
  config,
  pkgs,
  lib,
  ...
}: let
  cfg = config.services.ace-bot;
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
        ({pkgs, ...}: {
          environment.systemPackages = [cfg.shell];
        })
        ({pkgs, ...}: {
          # store settings
          nix.package = pkgs.nixVersions.latest; # for the local-overlay-store option
          nix.settings = {
            experimental-features = [
              "local-overlay-store"
              "read-only-local-store"
            ];
          };
          systemd.services."setup-nix-store" = {
            script = ''
              mount --types overlay overlay \
                --options lowerdir=/mnt/store-host/nix/store \
                --options upperdir=/mnt/store-disk/upper \
                --options workdir=/mnt/store-disk/work \
                --options userxattr \
                /nix/store

              mkdir --parents /root/.config/nix
              # lower store: a read only chroot store in /mnt/store-host
              cat >/root/.config/nix/nix.conf <<EOF
              store = local-overlay://?real=/nix/store&state=/nix/var&lower-store=/mnt/store-host?read-only=true&upper-layer=/mnt/store-disk/upper
              EOF
            '';
            path = with pkgs; [
              util-linux
            ];
            serviceConfig = {
              Type = "oneshot";
            };
            wantedBy = ["local-fs.target"];
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
in {
  options.services.ace-bot = {
    enable = lib.mkEnableOption "ace-bot";
    extraModules = lib.mkOption {
      type = with lib.types; listOf unspecified;
      default = [];
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
        CPUWeight = "idle";
        CPUQuota = "50%";
        MemoryMax = "1G";
        MemorySwapMax = "2G";
        LimitNProc = 2048;
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
        config.environment.etc."systemd/nspawn".source
      ];
    };
    systemd.services."ace-bot-image" = {
      wantedBy = ["systemd-nspawn@ace-bot.service"];
      before = ["systemd-nspawn@ace-bot.service"];
      partOf = ["systemd-nspawn@ace-bot.service"];
      script = ''
        set -x
        # setup disk
        if [ ! -f disk ]; then
          fallocate -l "${cfg.disk.size}" disk
          mkfs.ext4 disk
        fi
        mkdir --parents mount/disk
        mount disk mount/disk
        rm --recursive --force mount/disk/store/work
        mkdir --parents mount/disk/store/{upper,work,state}
        mkdir --parents mount/disk/home
        chown --recursive ace-bot:ace-bot mount/disk/home

        # setup image
        # just an empty directory
        mkdir /var/lib/machines/ace-bot
        # parents of mount points for overlay fs
        mkdir /var/lib/machines/ace-bot/nix
      '';
      postStop = ''
        set +e
        set -x
        umount mount/disk
        rmdir mount/disk
        rmdir mount
        rm --recursive --force /var/lib/machines/ace-bot
      '';
      path = with pkgs; [
        util-linux
        e2fsprogs
      ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        StateDirectory = "ace-bot";
        WorkingDirectory = "/var/lib/ace-bot";
      };
    };
    systemd.nspawn."ace-bot" = {
      execConfig = {
        Boot = false;
        Parameters = let
          initName =
            if envConfiguration.config.boot.initrd.systemd.enable
            then "prepare-root"
            else "init";
        in "${envConfiguration.config.system.build.toplevel}/${initName}";
        PrivateUsers = "${toString cfg.privateUsers.uidBase}:${toString cfg.privateUsers.gidBase}";
        LinkJournal = "host";
      };
      filesConfig = let
        disk = "/var/lib/ace-bot/mount/disk";
      in {
        PrivateUsersOwnership = "map";
        Bind = [
          "${disk}/store/state:/nix/var:idmap"
          "${disk}/store:/mnt/store-disk:idmap"
        ];
        BindReadOnly = [
          "/nix:/mnt/store-host/nix:idmap,norbind"
        ];
        OverlayReadOnly = [
          "+/mnt/store-host/nix/store:+/mnt/store-disk/upper:/nix/store"
        ];
        BindUser = ["ace-bot"];
      };
    };
    networking.firewall.allowedUDPPorts = [
      67 # DHCP server
    ];
  };
}
