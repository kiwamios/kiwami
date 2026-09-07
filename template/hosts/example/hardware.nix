# Hardware facts for this machine.
#
# A placeholder, so the template evaluates. `kiwami install` replaces it with
# what the machine actually reports:
#
#   nixos-generate-config --show-hardware-config --no-filesystems
#
# No UUIDs here on purpose: the mounts come from disk.nix, so they survive a
# reformat.
{ modulesPath, ... }:

{
  imports = [ (modulesPath + "/installer/scan/not-detected.nix") ];

  nixpkgs.hostPlatform = "x86_64-linux";

  boot.initrd.availableKernelModules = [ "xhci_pci" "nvme" "ahci" "sd_mod" ];
  boot.kernelModules = [ "kvm-intel" ];
}
