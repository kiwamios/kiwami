# Encrypted, and wiped at every boot.
#
# Both features existed before this host did, and neither had ever been used
# with the other. That gap hid a real defect: the rollback mounts the root
# filesystem in the initrd by device name, and on an encrypted machine that
# name is /dev/mapper/cryptroot rather than the partition - so it would have
# tried to mount a LUKS container as btrfs and dropped the machine into
# emergency mode on its first boot.
#
# A combination nothing exercises is a combination nobody has tested.
#
# The choices half of a machine: what it is called, who has an account, how it
# boots. The facts half is hardware.nix, and the shared Kiwami desktop comes
# from nixosModules.default, which mkHost adds to every host.
{ pkgs, ... }:

{
  imports = [
    ./hardware.nix
    ./disk.nix
  ];

  home-manager.users.nixos = {
    home.stateVersion = "26.05";
  };

  # The harness drives this machine over ssh and serial; there is nobody to
  # type a password at a greeter.
  kiwami.autoLogin = true;

  # No GPU here: llvmpipe pays for every frame in software, and animation also
  # makes the test screenshots nondeterministic - which is how the VM's answer
  # became the default for real hardware in the first place.
  kiwami.animations = false;

  # The point of this host: both at once.
  kiwami.ephemeralRoot = true;

  # The filesystem lives inside the container, so the rollback has to wait for
  # the container to be unlocked and then mount what came out of it. Naming
  # the partition here would reintroduce exactly the bug this host exists to
  # catch.
  kiwami.rootDevice = "/dev/mapper/cryptroot";

  # No splash on the test machines. Their console is how the harness knows
  # what happened, and a boot that looks nice to nobody is worth less than a
  # test that can read it.
  kiwami.splash.enable = false;

  kiwami.flake = "github:kiwamios/kiwami";
  networking.hostName = "kiwami-luks-ephemeral";

  boot.loader.systemd-boot.enable = true;
  # Must be true, or systemd-boot never registers an NVRAM entry and the
  # firmware drops to the EFI shell instead of booting. See vm/README.md.
  boot.loader.efi.canTouchEfiVariables = true;

  # Keep the serial console after install so the agent harness can drive the
  # machine with no graphical session.
  boot.kernelParams = [ "console=tty0" "console=ttyAMA0,115200" ];

  services.qemuGuest.enable = true;
  environment.systemPackages = [ pkgs.efibootmgr ];

  # Throwaway guest credentials. These must never appear on a real host.
  # No initialPassword: users are immutable, so the hash comes from
  # kiwami.passwordFile, which activation seeds with the default. Setting both
  # is a conflict Nix warns about, and the file wins.
  services.openssh.settings.PermitRootLogin = "yes";

  system.stateVersion = "26.05";
}
