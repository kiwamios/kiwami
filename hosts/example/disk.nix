# What the disk looks like.
#
# A placeholder, so the template evaluates. `kiwami install` replaces it with
# a layout for the disk you pick, and shows you the result before writing
# anything.
#
# disko formats from this, and fileSystems is derived from it, so the two
# cannot drift apart - which is why there are no UUIDs anywhere here and why
# reinstalling gives you the same mounts.
{ ... }:

{
  disko.devices.disk.system = {
    type = "disk";
    # Replaced by the installer with the disk you choose. By-id, not sda:
    # a device letter is a position in a queue, not an identity.
    device = "/dev/disk/by-id/REPLACE-ME";
    content = {
      type = "gpt";
      partitions = {
        ESP = {
          size = "1G";
          type = "EF00";
          content = {
            type = "filesystem";
            format = "vfat";
            mountpoint = "/boot";
            mountOptions = [ "umask=0077" ];
          };
        };
        root = {
          size = "100%";
          content = {
            type = "btrfs";
            extraArgs = [ "-f" ];
            subvolumes = {
              "@root" = {
                mountpoint = "/";
                mountOptions = [ "compress=zstd" "noatime" ];
              };
              "@nix" = {
                mountpoint = "/nix";
                mountOptions = [ "compress=zstd" "noatime" ];
              };
              "@persist" = {
                mountpoint = "/persist";
                mountOptions = [ "compress=zstd" "noatime" ];
              };
            };

            # The blank snapshot the initrd restores from, taken here because
            # this is the one moment @root is empty: disko has just created it
            # and nixos-install has not written to it yet. Without it an
            # ephemeral root has nothing to roll back to.
            postCreateHook = ''
              MNTPOINT=$(mktemp -d)
              mount "$device" "$MNTPOINT" -o subvol=/
              trap 'umount "$MNTPOINT"; rm -rf "$MNTPOINT"' EXIT
              btrfs subvolume snapshot -r "$MNTPOINT/@root" "$MNTPOINT/@root-blank"
            '';
          };
        };
      };
    };
  };
}
