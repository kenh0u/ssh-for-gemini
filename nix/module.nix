{ config, lib, pkgs, ... }:

let
  cfg = config.services.ssh-for-gemini;
in
{
  options.services.ssh-for-gemini = {
    enable = lib.mkEnableOption "SSH for Gemini pure HTTP POST/GET <-> local SSH bridge daemon";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.ssh-for-gemini;
      description = "The ssh-for-gemini package to run.";
    };

    listen = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1";
      description = "Address to bind the HTTP listener to.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 28787;
      description = "Port to bind the HTTP listener to.";
    };

    targetHost = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1";
      description = "Target TCP host to forward sessions to.";
    };

    targetPort = lib.mkOption {
      type = lib.types.port;
      default = 22;
      description = "Target TCP port to forward sessions to.";
    };

    maxSessions = lib.mkOption {
      type = lib.types.ints.positive;
      default = 16;
      description = "Maximum concurrent bridged sessions.";
    };

    idleTimeout = lib.mkOption {
      type = lib.types.ints.positive;
      default = 180;
      description = "Idle session timeout in seconds.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.ssh-for-gemini = {
      description = "SSH for Gemini (HTTP POST/GET <-> local SSH bridge)";
      after = [ "network.target" "sshd.service" ];
      wantedBy = [ "multi-user.target" ];

      serviceConfig = {
        ExecStart = lib.concatStringsSep " " [
          (lib.getExe cfg.package)
          "--listen" (lib.escapeShellArg cfg.listen)
          "--port" (toString cfg.port)
          "--target-host" (lib.escapeShellArg cfg.targetHost)
          "--target-port" (toString cfg.targetPort)
          "--max-sessions" (toString cfg.maxSessions)
          "--idle-timeout" (toString cfg.idleTimeout)
        ];
        Restart = "always";
        RestartSec = 2;
        DynamicUser = true;
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" ];
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
      };
    };
  };
}
