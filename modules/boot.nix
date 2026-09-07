# What the machine looks like before you have logged into it.
#
# A NixOS machine boots by printing several screens of kernel and systemd
# output, then asking for a disk passphrase on a bare console. Both are
# accurate and neither is something anybody wants to watch every morning.
#
# The whole of this file is presentation. Nothing here changes what the
# machine does - `kiwami.splash.enable = false` and you get the log back,
# which is exactly what you want the day something fails early.
{ config, lib, pkgs, ... }:

let
  cfg = config.kiwami.splash;

  # Plymouth wants a raster image, and the mark is drawn as vectors so it
  # stays sharp on any panel. Converted at build time rather than a PNG
  # committed to the repository, so the two cannot drift apart.
  logo = pkgs.runCommand "kiwami-boot-logo.png" { } ''
    ${pkgs.librsvg}/bin/rsvg-convert -w 256 -h 256 ${../config/plymouth/logo.svg} -o $out
  '';
in
{
  config = lib.mkIf cfg.enable {
    # Plymouth is a small program that runs inside the initrd, before the real
    # system exists. It draws straight to the graphics device, which is why it
    # can do this and a Wayland shell cannot: at this point there is no
    # compositor, no /nix/store and no home directory.
    #
    # It also draws the disk passphrase prompt, which is the other half of the
    # problem - that prompt is otherwise a bare line on a text console.
    boot.plymouth = {
      enable = true;
      theme = lib.mkDefault cfg.theme;
      logo = lib.mkDefault logo;
    };

    # Quiet, but not silent-and-unrecoverable.
    #
    # loglevel=3 keeps warnings and errors; it drops the informational stream
    # that scrolls past. A machine that fails early still says why, and
    # `kiwami.splash.enable = false` puts everything back.
    # "splash" and "loglevel" are not here: the plymouth module already adds
    # both, and listing them again put each on the kernel command line twice.
    boot.kernelParams = [
      "quiet"
      "rd.udev.log_level=3"
      "udev.log_priority=3"
      # The cursor blinking in the corner of an otherwise composed splash.
      "vt.global_cursor_default=0"
    ];
    boot.consoleLogLevel = lib.mkDefault 3;
    boot.initrd.verbose = lib.mkDefault false;

    # The generation menu, hidden but not gone.
    #
    # systemd-boot shows it anyway if a key is held during boot, so this costs
    # nothing in recovery: booting the previous generation is still how you
    # fix a machine that comes up broken, and that has to keep working
    # precisely because everything else here is cosmetic.
    boot.loader.timeout = lib.mkDefault 0;
  };
}
