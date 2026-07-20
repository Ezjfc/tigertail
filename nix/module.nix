# NixOS module for chiral. No udev/hidraw rules are needed: pen events are read
# from the tablet over SSH and injected through the Wayland wlr-virtual-pointer
# protocol, so nothing touches local /dev nodes.
self: { config, lib, pkgs, ... }:
let
  cfg = config.programs.chiral;
in
{
  options.programs.chiral = {
    enable = lib.mkEnableOption "chiral, reMarkable 2 drawing-tablet bridge";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.chiral;
      defaultText = lib.literalExpression "chiral.packages.\${system}.chiral";
      description = "The chiral package to install.";
    };

    remarkableHost = lib.mkOption {
      type = lib.types.str;
      default = "10.11.99.1";
      description = "Address of the reMarkable tablet (10.11.99.1 over USB).";
    };

    sshHostAlias = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = "remarkable";
      description = ''
        Adds a system-wide ssh_config Host block with this alias pointing at
        the tablet (user root), so `ssh remarkable` and `ssh-copy-id remarkable`
        work out of the box. Set to null to skip.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    programs.ssh.extraConfig = lib.mkIf (cfg.sshHostAlias != null) ''
      Host ${cfg.sshHostAlias}
        HostName ${cfg.remarkableHost}
        User root
    '';
  };
}
