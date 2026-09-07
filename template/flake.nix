# My machines.
#
# Kiwami is the distro; this repository is what is yours about your particular
# computers: their names, their disks, their hardware, where they back up to.
# Kiwami has no idea any of it exists, which is the point - it can be updated,
# or forked, or handed to somebody else, without carrying your laptop with it.
#
# Adding a machine is `kiwami install`, which scaffolds hosts/<name>/ and
# offers to push it here. Nothing in this file has to be edited to register
# it: the host list is the directory listing.
{
  description = "My machines";

  inputs = {
    kiwami.url = "github:kiwamios/kiwami";
  };

  outputs = { self, kiwami, ... }:
    let
      lib = kiwami.inputs.nixpkgs.lib;
      hostNames = builtins.attrNames
        (lib.filterAttrs (_: t: t == "directory") (builtins.readDir ./hosts));
    in
    {
      # mkHost comes from Kiwami rather than being rebuilt here, so these
      # machines are assembled exactly the way the distro's own test machines
      # are. A recipe copied by hand is a recipe that drifts.
      nixosConfigurations =
        lib.genAttrs hostNames (name: kiwami.lib.mkHost [ (./hosts + "/${name}") ])
        # An installer per machine, carrying that machine's whole built system,
        # so a reinstall needs no network. `kiwami image` builds
        # installer-<host> from this flake, so the attribute has to exist here.
        //
        lib.genAttrs (map (n: "installer-${n}") hostNames)
          (imageName:
            kiwami.lib.installerFor
              self.nixosConfigurations.${lib.removePrefix "installer-" imageName});
    };
}
