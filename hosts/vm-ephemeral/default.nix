# The ephemeral-root rehearsal machine.
#
# Same desktop as the dev VM, with a root that is wiped at every boot. It
# exists to be reinstalled and rebooted repeatedly until the persist list
# stops losing things - which is not a rehearsal you want to run on a laptop.
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

  # The point of this host.
  kiwami.ephemeralRoot = true;

  kiwami.flake = "github:kiwamios/kiwami";
  networking.hostName = "kiwami-ephemeral";

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
