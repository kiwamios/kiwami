# xps
#
# Scaffolded by `kiwami install`. Everything here is a choice, not a fact -
# edit it freely, then `nixos-rebuild switch --flake .#xps`.
#
# hardware.nix beside this file is the facts half, detected at install time.
# Regenerate it with `kiwami doctor` if this machine's hardware changes.
{ ... }:

{
  imports = [
    # Detected at install time. Do not edit; `kiwami doctor` diffs it against
    # what this machine currently reports.
    ./hardware.nix
    # The layout you chose. disko formats from it and fileSystems is derived
    # from it, so the two cannot drift apart.
    ./disk.nix
  ];

  kiwami.flake = "github:kiwamios/kiwami";
  networking.hostName = "xps";

  # The account the desktop belongs to. greetd logs this user in and its home
  # carries the Hyprland and Quickshell config, so a mismatch here is quiet
  # and total: the session comes up on Hyprland's own default config with no
  # bar, because everything was installed for a different account.
  kiwami.user = "kiwami";

  # The root is wiped at every boot; only what kiwami.persist declares
  # survives. disk.nix beside this makes the subvolumes that depend on.
  kiwami.ephemeralRoot = true;

  # The filesystem lives inside the LUKS container, so the rollback has to
  # mount what comes out of it rather than the partition, which holds
  # ciphertext. Without this the initrd would try to mount a LUKS blob as
  # btrfs and the root would silently stop being wiped - or fail to boot.
  kiwami.rootDevice = "/dev/mapper/cryptroot";

  # Log the desktop user in with no password. Off deliberately: it suits a
  # throwaway VM and not a laptop, where it means whoever opens the lid is
  # you. Uncomment only if you know that is what you want.
  # kiwami.autoLogin = true;

  # /persist, encrypted, in R2. The prefix is what keeps one bucket usable for
  # more than this machine: another host would be .../kiwami-<name> and a
  # separate repository with its own passphrase, sharing nothing. restic has
  # no per-host permissions inside a repository, so separate is the safer
  # default even between machines you own.
  kiwami.backup.enable = true;
  kiwami.backup.repository =
    "s3:https://4e11898df61f8103785b672c02504096.r2.cloudflarestorage.com/kiwami-backups/kiwami-xps";

  boot.loader.systemd-boot.enable = true;
  # Without this systemd-boot never registers an NVRAM entry and the firmware
  # boots something else, or nothing.
  boot.loader.efi.canTouchEfiVariables = true;

  # The account itself comes from modules/common.nix, which reads
  # kiwami.user above. The password is not set here: users are immutable, so
  # the hash comes from kiwami.passwordFile, which activation seeds with the
  # default and `kiwami passwd` replaces. Setting initialPassword as well is a
  # conflict Nix only warns about, and the file wins - so the line would look
  # like it set the password while doing nothing.

  home-manager.users.kiwami = {
    imports = [
      ../../modules/home/configs.nix
      ../../modules/home/shell.nix
    ];
    home.stateVersion = "26.05";
  };

  system.stateVersion = "26.05";
}
