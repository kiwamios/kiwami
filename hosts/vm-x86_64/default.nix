# x86_64 QEMU guest. Same desktop as the aarch64 dev VM, different
# architecture, so CI catches anything that only breaks on x86_64.
#
# Nothing runs this interactively; it exists to be built and booted in CI.
{ pkgs, ... }:

{
  imports = [
    ./hardware.nix
    ./disk.nix
  ];

  home-manager.users.nixos = {
    imports = [
      ../../modules/home/configs.nix
      ../../modules/home/shell.nix
    ];
    home.stateVersion = "26.05";
  };

  # The harness drives this machine over ssh and serial; there is nobody to
  # type a password at a greeter.
  kiwami.autoLogin = true;

  # No GPU here: llvmpipe pays for every frame in software, and animation also
  # makes the test screenshots nondeterministic - which is how the VM's answer
  # became the default for real hardware in the first place.
  kiwami.animations = false;

  # Predates the ephemeral layout and still has a single ext4 root. Not a
  # supported mode - converting these is follow-up work.
  kiwami.ephemeralRoot = false;

  kiwami.flake = "github:kiwamios/kiwami";
  networking.hostName = "kiwami-vm-x86";

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;

  boot.kernelParams = [ "console=tty0" "console=ttyS0,115200" ];

  services.qemuGuest.enable = true;
  environment.systemPackages = [ pkgs.efibootmgr ];

  # No initialPassword: users are immutable, so the hash comes from
  # kiwami.passwordFile, which activation seeds with the default. Setting both
  # is a conflict Nix warns about, and the file wins.

  system.stateVersion = "26.05";
}
