# An example machine. Rename the directory to whatever this computer is
# called, or delete it once `kiwami install` has scaffolded a real one.
#
# Everything here is a choice, not a fact. The facts - what disks exist, what
# hardware is present - live in the two files beside this, and the installer
# writes those for you.
{
  imports = [
    ./hardware.nix
    ./disk.nix
  ];

  networking.hostName = "example";

  # The account the desktop belongs to. greetd logs this user in and its home
  # carries the Hyprland and Quickshell config, so a mismatch here is quiet
  # and total: the session comes up with no bar, because everything was
  # installed for somebody else.
  kiwami.user = "you";

  # Where this machine rebuilds itself from. It keeps no checkout, so without
  # this `kiwami update` has nowhere to build - point it at this repository.
  kiwami.flake = "github:you/my-machines";

  # The root is wiped at every boot; only what kiwami.persist declares
  # survives. disk.nix beside this makes the subvolumes that depend on.
  kiwami.ephemeralRoot = true;

  # Backups: /persist is the whole target. `kiwami snapshot setup` fills in
  # the credentials; this only says where they go.
  # kiwami.backup.enable = true;
  # kiwami.backup.repository = "s3:https://<account>.r2.cloudflarestorage.com/<bucket>/<host>";

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;

  # When this machine was installed. Kiwami applies the desktop user's home
  # configuration itself; this is the one part of it that belongs to the
  # machine rather than to the distro.
  home-manager.users.you.home.stateVersion = "26.05";

  system.stateVersion = "26.05";
}
