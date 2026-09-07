# Settings every Kiwami host gets, regardless of hardware.
{ config, lib, pkgs, inputs, ... }:

let
  # Where the hash is during *activation*, which is not where it is at
  # runtime.
  #
  # Activation runs in the initrd, before the bind mounts that graft /persist
  # into place. So /var/lib/kiwami/passwords - the runtime path - is an empty
  # directory on the bare root at the moment the account is created, and
  # naming it there meant the seed below fired every boot and the default hash
  # went into /etc/shadow. Until something re-ran activation, the machine's
  # password was the one published in this repository, and the owner's own
  # password was rejected.
  #
  # /persist is neededForBoot, so it is mounted by then. Naming the file
  # through /persist makes it the same file at both moments - the bind mount
  # just gives it a second name later - and there is exactly one copy.
  passwordDir =
    if config.kiwami.ephemeralRoot
    then "/persist${config.kiwami.passwordFile}"
    else config.kiwami.passwordFile;
in
{
  nix.settings = {
    experimental-features = [ "nix-command" "flakes" ];
    auto-optimise-store = true;
  };

  time.timeZone = lib.mkDefault "Europe/Zurich";
  i18n.defaultLocale = lib.mkDefault "en_US.UTF-8";

  # One way to set a password, everywhere. `passwd` writes to /etc/shadow,
  # which an ephemeral root discards while reporting success - so rather than
  # supporting two modes and explaining when each applies, users are immutable
  # on every machine and the hash comes from a file. nixpkgs drops the setuid
  # passwd wrapper when mutableUsers is off, so the wrong command is absent
  # rather than merely wrong.
  users.mutableUsers = false;

  # Make sure the hash file exists before users are created. With immutable
  # users a missing hashedPasswordFile does not fall back to anything - the
  # account is simply never given a password and cannot log in. The installer
  # seeds it, but a machine built any other way (the CI boot test, or an
  # install where seeding failed) would come up with a user nobody can be.
  #
  # The default is a fixed hash of "kiwami"; `kiwami doctor` reports it as the
  # install default until `kiwami passwd` replaces it.
  system.activationScripts.kiwamiPassword = {
    deps = [ "specialfs" ];
    text = ''
      mkdir -p ${passwordDir}
      chmod 700 ${passwordDir}
      if [ ! -s ${passwordDir}/${config.kiwami.user} ]; then
        echo '$6$kiwamidefault$RHqPdZfAbfcBgynCC4GyrLHRK4DT0IXCI6QwVxObCgTY9Ky6dUSfFpyhBLvBuTozVnGeXnNSczef4HvLQPy1U1' \
          > ${passwordDir}/${config.kiwami.user}
        chmod 600 ${passwordDir}/${config.kiwami.user}
      fi
    '';
  };
  system.activationScripts.users.deps = [ "kiwamiPassword" ];

  users.users.${config.kiwami.user} = {
    isNormalUser = true;
    extraGroups = [ "wheel" "video" "audio" "networkmanager" ];
    # Written by `kiwami install`, changed by `kiwami passwd`. Without it the
    # account has no password at all - and with immutable users there is no
    # initialPassword fallback, so that is discovered at a greeter.
    hashedPasswordFile = "${passwordDir}/${config.kiwami.user}";
  };
  security.sudo.wheelNeedsPassword = false;

  # Compressed swap in RAM, since there is no swap partition: hibernation is
  # the only reason to want one, and it is not available with a root that gets
  # rolled back. Without this there is no swap at all, and the kernel's only
  # way to free memory is evicting the page cache - throwing out the
  # executables it is running from, then reaching for the OOM killer sooner.
  #
  # Cold pages compress well, so this holds appreciably more than it costs.
  # It is also pure configuration: no partition, and a rebuild away from being
  # changed or joined by a swapfile.
  zramSwap.enable = true;

  # State this distro knows it needs. Everything here was learned the hard
  # way or is a well-known trap:
  #
  #   ssh host keys      - regenerate and every client warns about a changed
  #                        key, which is indistinguishable from an attack
  #   /var/lib/nixos     - uid and gid allocation. Lose it and a rebuilt user
  #                        can get a different uid than the files they own
  #   /var/lib/systemd   - machine-id, which journald and much else key on
  #   NetworkManager     - the wifi networks you have joined, and separately
  #                        NM's own state: leases and seen-bssids. Declaring
  #                        only the profiles leaves it half-remembering
  #                        networks. Found by the doctor check, not by me
  #   tailscale          - the node identity; losing it means a fresh browser
  #                        login on a machine that may have no browser
  #   /var/lib/kiwami    - the keyfile that opens a second encrypted disk
  kiwami.persist.directories = [
    "/var/lib/nixos"
    "/var/lib/systemd"
    "/var/lib/kiwami"
    "/etc/NetworkManager/system-connections"
    "/var/lib/NetworkManager"
  ] ++ lib.optional config.services.tailscale.enable "/var/lib/tailscale";

  kiwami.persist.files = [
    "/etc/machine-id"
    "/etc/ssh/ssh_host_ed25519_key"
    "/etc/ssh/ssh_host_ed25519_key.pub"
    "/etc/ssh/ssh_host_rsa_key"
    "/etc/ssh/ssh_host_rsa_key.pub"
  ];

  # The home paths that survive a wipe.
  #
  # Listed here in full rather than left to the option's default: a `default`
  # is discarded the moment any module defines a value, so adding ".config/gh"
  # on its own silently dropped ~/.ssh, ~/Projects and ~/kiwami - the checkout
  # the machine rebuilds itself from. Nothing failed; the paths simply stopped
  # being bound, and would have been gone at the next boot.
  #
  # No ~/kiwami. The machine rebuilds from the flake on GitHub - `kiwami
  # update` - and keeps no copy of its own configuration.
  #
  # Every bug in this area came from having one: a copy that could be older
  # than GitHub, older than the disk it describes, or restored from a backup
  # taken before a layout change, each able to revert the machine's config
  # silently at the next rebuild. None of that is possible when the copy does
  # not exist. Hacking on the config is a clone in ~/Projects like any other
  # repository, which is a workspace rather than a second source of truth.
  #
  # ~/.local/state holds the applied theme and other session state.
  #
  # ~/Projects is work in progress: mostly in git and mostly pushed, but the
  # part that matters is the part that is not.
  #
  kiwami.persist.userDirectories = [
    ".ssh"
    ".local/state"
    "Projects"
    # Wallpapers live under here, and the root is wiped at every boot:
    # without this you add one, set it, and find an empty directory in the
    # morning with no error anywhere to explain it.
    #
    # Kiwami's own directory rather than ~/Pictures, which is load-bearing
    # for other things: persisting that would drag a whole photo library
    # into /persist, and from there into every backup.
    ".local/share/kiwami"
  ];

  # gh's token, and deliberately nothing else from that directory. config.yml
  # is written by the flake and state.yml is an update-check cache that should
  # not outlive a boot, so each path in there has exactly one owner.
  kiwami.persist.userFiles = [ ".config/gh/hosts.yml" ];

  # NetworkManager, not scripted DHCP. `kiwami net` drives nmcli, so without
  # this the command ships on every machine and works on none of them - and a
  # laptop with no way to join a wireless network is not a laptop. The VM
  # never noticed, having only ever had a wired connection that came up by
  # itself.
  networking.networkmanager.enable = lib.mkDefault true;
  # NetworkManager owns the interfaces; leaving the scripted DHCP client on as
  # well means two things bringing up the same link.
  networking.useDHCP = lib.mkDefault false;

  # The daemon, running but joined to nothing. `tailscale up` - which is what
  # `kiwami remote` calls - is still an explicit act, so this grants no access
  # by itself; it only means the machine can be reached for help without
  # first having to install something on a machine that has no network.
  services.tailscale.enable = lib.mkDefault true;

  services.openssh = {
    enable = true;
    settings.PasswordAuthentication = lib.mkDefault true;
  };

  environment.systemPackages = [
    # The distro's own CLI, built from cli/ by this flake.
    inputs.self.packages.${pkgs.stdenv.hostPlatform.system}.kiwami

    # And passwd taken away. Immutable users drop its setuid wrapper, so an
    # unprivileged passwd fails loudly - but `sudo passwd` still reports
    # "password updated successfully" and is reverted at the next activation.
    # That was verified, not assumed. Detecting it afterwards is worse than
    # not shipping the footgun, so the binary is shadowed by one that says
    # where to go instead. hiPrio wins the collision with the shadow package,
    # which is still needed for su and friends.
    (lib.hiPrio (pkgs.writeShellScriptBin "passwd" ''
      echo "passwd does nothing here: users are immutable, so the hash comes" >&2
      echo "from a file and /etc/shadow is regenerated at activation." >&2
      echo >&2
      echo "  sudo kiwami passwd" >&2
      exit 1
    ''))
  ] ++ (with pkgs; [
    git
    vim
    curl
    htop
    rsync
    jq

    # gh, because a machine that can build its config but not push it loses
    # the config when the disk dies - and that is not hypothetical, it is how
    # hosts/xps spent its first week. gh authenticates through a browser on
    # whatever device you have, so there is no key to place first.
    gh

    # python3 as a tool, not a runtime: one-liners and small scripts,
    # including remote maintenance on a machine whose only other option is
    # sed. Project dependencies belong in a per-project devshell with uv, so
    # two projects cannot collide in the system profile.
    python3
  ]);
}
