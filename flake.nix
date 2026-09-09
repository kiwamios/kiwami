{
  description = "Kiwami — a personal NixOS desktop";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    home-manager = {
      url = "github:nix-community/home-manager/release-26.05";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Declarative disk layout. The installer writes a disk.nix per machine and
    # disko both formats from it and derives fileSystems, so the layout is
    # stated once instead of once in Rust and once in Nix.
    disko = {
      url = "github:nix-community/disko";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Bind-mounts declared paths back over an ephemeral root. Hand-rolling
    # this is where impermanence setups go subtly wrong - directories that
    # need creating first, files that cannot be bind-mounted onto nothing,
    # ownership that has to survive - so the well-solved version is used.
    impermanence.url = "github:nix-community/impermanence";

    # The desktop shell. Pinned deliberately: Quickshell is alpha and ships
    # breaking QML API changes, so it must move when we say so, not when a
    # distro packager pushes.
    # A terminal workspace manager, not in nixpkgs - it ships its own flake.
    #
    # Pinned to a release tag rather than master: the docs recommend it, and a
    # terminal multiplexer moving under you is not the kind of surprise
    # anybody wants mid-session. It builds from source in about four minutes
    # and is not in the public cache, so a version bump costs that on each
    # machine.
    herdr = {
      url = "github:herdrdev/herdr/v0.9.0";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    quickshell = {
      url = "github:quickshell-mirror/quickshell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, home-manager, ... }@inputs:
    let
      systems = [ "aarch64-linux" "x86_64-linux" "aarch64-darwin" ];
      forAllSystems = f:
        nixpkgs.lib.genAttrs systems (s: f nixpkgs.legacyPackages.${s});

      # No `system` argument: every host's hardware.nix sets
      # nixpkgs.hostPlatform, which is where nixosSystem takes it from. Saying
      # it twice means a machine whose architecture can disagree with its own
      # hardware description.
      mkHost = modules:
        nixpkgs.lib.nixosSystem {
          # Makes every flake input reachable inside modules as `inputs.*`.
          specialArgs = { inherit inputs; };
          modules = modules ++ [
            # Our own hosts are built from the same module we export, so
            # anything that breaks for a consumer breaks for us first.
            self.nixosModules.default
            inputs.disko.nixosModules.disko
            # Set here rather than in a shared module: nixosTest supplies its
            # own pkgs instance and rejects a config being set from inside.
            { nixpkgs.config.allowUnfree = true; }
          ];
        };

      # Every directory under hosts/ is a machine. Adding one is creating a
      # folder and `git add`-ing it - there is no second list to keep in sync,
      # which matters because `kiwami install` scaffolds these directories and
      # nothing should have to edit this file to register them.
      hostNames = builtins.attrNames
        (nixpkgs.lib.filterAttrs (_: t: t == "directory") (builtins.readDir ./hosts));

      # The harness key, present only in the -test images. A shipped
      # installer must not carry a key from this repository; the test
      # variant carries it so `vmssh` works and the whole installer matrix
      # runs against the real image instead of being rewritten to drive a
      # serial console.
      # Missing is an error, not an empty list. A -test image whose entire
      # purpose is carrying this key built happily without one, and the
      # only symptom was ssh refusing a connection several minutes later.
      harnessKey =
        let f = ./vm/keys/kiwami_vm.pub;
        in if builtins.pathExists f
           then [ (builtins.readFile f) ]
           else throw ''
             installer-*-test needs vm/keys/kiwami_vm.pub, which is not in
             this flake. Generate it with `just vm install`, and remember
             that flakes only see git-tracked files.
           '';

      # An installer image carrying a host's entire built system.
      #
      # `nixos-install --system <path>` installs a prebuilt closure rather
      # than evaluating a flake, so a machine can be rebuilt with no
      # network at all: the 5.7 GiB it would otherwise download is already
      # on the stick. What it does not carry is any state - no keys, no
      # tokens, no wifi - so the image is not a bundle of secrets. It is
      # still your configuration, which names your bucket and your
      # hostname, so it is private rather than publishable.
      #
      # Deliberately not fresh-by-design: the image supplies the machine,
      # `kiwami snapshot restore` supplies what has happened since. A
      # months-old image still boots you into a working laptop.
      mkInstaller = { system, testKey ? false, prebuilt ? null }:
    nixpkgs.lib.nixosSystem {
      specialArgs = { inherit inputs; };
      modules = [
        ({ modulesPath, pkgs, lib, ... }: {
          imports = [
            (modulesPath + "/installer/cd-dvd/installation-cd-minimal.nix")
            # The console, and only the console. The installer carries none of
            # the rest of Kiwami - no desktop, no impermanence - but the screen
            # you install from should look like the system you are installing.
            ./modules/console.nix
          ];

          nixpkgs.hostPlatform = system;

          # The whole point: `kiwami install`, not a nix run incantation.
          # git comes too - installing a new machine needs a writable
          # checkout to generate hardware.nix into.
          environment.systemPackages = [
            self.packages.${system}.kiwami
            pkgs.git
            # `kiwami remote` drives this. Present but not started: joining
            # a tailnet is an explicit act, not something live media should
            # do on its own. It is also how the backup credentials reach
            # this machine, by taildrop or scp, without anything long being
            # typed.
            pkgs.tailscale
            # The installer offers to restore a previous machine's identity
            # before the first boot, which is the thing that makes a
            # reinstall cheap. Without restic here that offer would fail at
            # the last possible moment, after the disk was already erased.
            pkgs.restic
            # fix_boot_order reads and rewrites NVRAM from here rather than
            # from inside the target, so the installed system does not have
            # to have chosen to ship it - which, on real hardware, it had
            # not.
            pkgs.efibootmgr
            # The installer offers to push the host it just wrote, which
            # means logging in to GitHub from here. Without gh on the image
            # that offer would fail at the moment it was accepted.
            pkgs.gh
            # How the credentials get here without anything long being
            # typed: one note holds the whole environment file. Unlocking a
            # vault exposes all of it to whatever runs on the machine,
            # which is a real objection on a daily driver and a small one
            # on live media that is minutes from being wiped.
            pkgs.bitwarden-cli
          ];

          systemd.services.tailscaled = {
            description = "Tailscale, started on demand by `kiwami remote`";
            wantedBy = lib.mkForce [ ];
            serviceConfig.ExecStart = "${pkgs.tailscale}/bin/tailscaled";
          };

          users.users.nixos.openssh.authorizedKeys.keys =
            lib.optionals testKey harnessKey;

          # The harness drives this image by typing at the serial console,
          # which means it needs a shell there - and the installer now
          # takes that console for itself, which is right for a headless
          # machine and fatal for a test that has to run `kiwami install`
          # with flags.
          #
          # Pre-creating the marker the autostart already checks turns it
          # off without a second mechanism to keep in step. Only on the
          # -test image: the real one must start on its own, which is the
          # whole point of it.
          systemd.tmpfiles.rules =
            lib.optional testKey "f /tmp/.kiwami-installer-started 0644 root root -";

          # The installer shells out to `nix build` for the disko script,
          # and the stock ISO does not enable flakes.
          nix.settings.experimental-features = [ "nix-command" "flakes" ];

          # The host's closure, and a signpost saying it is there. Two
          # files rather than one so the installer can refuse to use a
          # closure built for a different machine.
          isoImage.storeContents =
            lib.optional (prebuilt != null) prebuilt.config.system.build.toplevel;
          isoImage.contents = lib.optionals (prebuilt != null) [
            {
              source = pkgs.writeText "kiwami-host" prebuilt.config.networking.hostName;
              target = "/kiwami/host";
            }
            {
              source = pkgs.writeText "kiwami-system"
                "${prebuilt.config.system.build.toplevel}";
              target = "/kiwami/system";
            }
          ];

          # zstd, not the default xz: the image is recompressed in full for
          # every change, however small, and that cost dominates the build.
          # A throwaway image booted in QEMU does not care about its size.
          isoImage.squashfsCompression = "zstd -Xcompression-level 3";

          # Long enough to actually catch. The default hurried past too
          # fast to pick anything from the boot menu.
          boot.loader.timeout = 10;

          # Keep the serial console so the harness can drive the image
          # exactly as it drives the stock ISO.
          #
          # copytoram reads the squashfs into memory once, at boot, and
          # runs from there. Without it the whole live system is paged off
          # the USB stick for the length of the install, and a link that
          # drops - a portable SSD renegotiating power, a marginal USB-C
          # port - takes everything with it: squashfs I/O errors, then
          # processes segfaulting because their pages cannot be read back.
          # Seen on an XPS 13 mid-install.
          #
          # It costs the image's size in RAM (1.5G here) and a slower boot,
          # which is one long sequential read instead of thousands of small
          # random ones under load. That trade is worth taking by default;
          # a machine too small for it is not one this installs on.
          boot.kernelParams =
            [ "console=tty0" "copytoram" ]
            ++ lib.optional (system == "aarch64-linux") "console=ttyAMA0,115200"
            ++ lib.optional (system == "x86_64-linux") "console=ttyS0,115200";

          # Start the installer on login, on the first console only.
          #
          # An image whose purpose is installing should not require
          # remembering three commands in the right order. But it must be
          # escapable: the marker means quitting drops you to a shell and
          # a second login stays a shell, so a wrong turn is not a reboot.
          # Other TTYs are left alone entirely.
          programs.bash.loginShellInit = ''
            # Where the installer should appear: the serial console when
            # the kernel has one - a headless machine, or the harness -
            # and the screen otherwise. Exactly one console, because two
            # installers on one disk is worse than none.
            #
            # Gating on tty1 alone put it where nobody was looking: a
            # headless install ran the whole conversation on a screen that
            # does not exist, while the serial line it was being watched
            # on sat at a shell prompt. ssh gets /dev/pts/N and matches
            # neither.
            kiwami_want=/dev/tty1
            for kiwami_t in $(cat /sys/class/tty/console/active 2>/dev/null); do
              case "$kiwami_t" in
                ttyS*|ttyAMA*) kiwami_want=/dev/$kiwami_t ;;
              esac
            done
            if [ "$(tty)" = "$kiwami_want" ] && [ ! -e /tmp/.kiwami-installer-started ]; then
              touch /tmp/.kiwami-installer-started
              sudo kiwami install --guided || true
              echo
              echo "Installer exited. You are at a shell; run it again with:"
              echo "  sudo kiwami install --guided"
            fi
          '';

          services.getty.helpLine = lib.mkForce ''

            Kiwami installer. It starts on its own; these are for when
            you have left it, or are on another console.

              sudo kiwami install --guided    the whole thing: network, remote, install
              sudo kiwami net                 just get online
              sudo kiwami remote              just be reachable over your tailnet

            Machines live in a repository you control, with Kiwami as an
            input. If you do not have one yet, on any machine with nix:

              nix flake init -t github:kiwamios/kiwami

            Then, here:

              sudo kiwami install --flake github:you/your-repo --host <name> --new
          '';
        })
      ];
    };
    in
    {
      # The kiwami CLI. Built once here and consumed by the hosts below, so the
      # binary that ships on an image is the same one `nix run` gives you.
      packages = forAllSystems (pkgs: rec {
        kiwami = pkgs.rustPlatform.buildRustPackage {
          pname = "kiwami";
          version = "0.1.0";
          src = ./cli;
          cargoLock.lockFile = ./cli/Cargo.lock;
          meta.mainProgram = "kiwami";

          nativeBuildInputs = [ pkgs.makeWrapper ];
          # `kiwami install` shells out to these. On the installer ISO they
          # happen to be present; everywhere else they are not, and the
          # installer would fail partway through with "command not found"
          # after it had already started writing. Carry them explicitly.
          #
          # parted, mkfs and mount are gone: disko's generated script carries
          # its own dependencies, so the installer runs none of them itself.
          #
          # nmcli is deliberately NOT here. It is a client for a running
          # NetworkManager daemon, so bundling it would add NM's whole closure
          # to every desktop for a binary that is useless without the service
          # - and a bundled client can skew from the daemon it talks to.
          # `kiwami net` looks it up on PATH and says so when it is missing.
          postInstall = ''
            wrapProgram $out/bin/kiwami --prefix PATH : ${pkgs.lib.makeBinPath (with pkgs; [
              curl            # the "am I online" probe
              git             # staging the generated hardware.nix
              mkpasswd        # hashing for `kiwami passwd`
            ])}
          '';
        };
        default = kiwami;
      });

      # Everything that needs a Linux builder. See tests/ - nixosTest cannot
      # run on the Mac at all, which is why these gate CI rather than the
      # local loop.
      checks = forAllSystems (pkgs:
        pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux
          (import ./tests { inherit pkgs inputs home-manager; }));

      # How a machine is built from Kiwami, exported so a consumer does not
      # have to reconstruct it.
      #
      # Without this, somebody keeping their hosts in their own repository has
      # to rediscover the whole recipe - nixosSystem with our module, disko,
      # inputs passed through specialArgs, allowUnfree - and gets a subtly
      # different machine from ours whenever they miss a piece. Our own hosts
      # are built through this same function, so their machines and ours
      # cannot drift.
      #
      #   nixosConfigurations.thinkpad = kiwami.lib.mkHost [ ./hosts/thinkpad ];
      lib = {
        inherit mkHost;

        # An installer image for a host defined somewhere else.
        #
        #   installer-thinkpad = kiwami.lib.installerFor
        #     self.nixosConfigurations.thinkpad;
        #
        # `kiwami image` builds installer-<host> from the machine's own flake,
        # so a consumer keeping hosts in their own repository needs to be able
        # to produce that attribute. Without this the command works for this
        # repository and nowhere else - which is the same mistake the
        # hardcoded flake URL was.
        installerFor = target: mkInstaller {
          system = target.config.nixpkgs.hostPlatform.system;
          prebuilt = target;
        };

        # A plain installer for an architecture, for a machine that does not
        # exist yet.
        installerFallback = system: mkInstaller { inherit system; };
      };

      # A repository to keep your own machines in.
      #
      #   nix flake init -t github:kiwamios/kiwami
      #
      # It lives in this repository rather than in documentation so that it is
      # evaluated with everything else: if mkHost changes shape, the template
      # breaks here, in a build, rather than in a stranger's first hour.
      templates.default = {
        path = ./template;
        description = "A repository describing your machines, built from Kiwami";
      };

      # The distro as an importable module, so a machine can be built from
      # Kiwami without forking it. Deliberately excludes anything
      # host-specific - hardware, hostname, users - which the consumer
      # supplies alongside it.
      nixosModules.default = { config, lib, ... }: {
        imports = [
          home-manager.nixosModules.home-manager
          ./modules/apps.nix
          ./modules/boot.nix
          ./modules/common.nix
          ./modules/console.nix
          ./modules/desktop.nix
          ./modules/options.nix
          ./modules/themes.nix
          ./modules/generated.nix
          ./modules/impermanence.nix
          ./modules/backup.nix
        ];

        # Modules reach the flake's own inputs (the kiwami package, quickshell)
        # through this rather than the consumer having to thread it.
        _module.args.inputs = inputs;

        nixpkgs.config.allowUnfree = lib.mkDefault true;

        home-manager.useGlobalPkgs = lib.mkDefault true;
        home-manager.useUserPackages = lib.mkDefault true;
        home-manager.extraSpecialArgs = { inherit inputs; };

        # The desktop user's home configuration, applied by the distro rather
        # than imported by each host.
        #
        # Every host used to carry `../../modules/home/configs.nix` - a
        # relative path into this repository, which resolves only for a host
        # that lives inside it. The moment hosts moved to their own flake the
        # path pointed at nothing, and the machine could not be built at all.
        # It was boilerplate repeated in six files that also happened to be
        # the one line making the module non-portable.
        #
        # home.stateVersion stays with the host: it is a fact about when that
        # particular machine was installed, not a choice this distro makes.
        home-manager.users.${config.kiwami.user}.imports = [
          ./modules/home/configs.nix
          ./modules/home/shell.nix
        ];
      };

      nixosConfigurations =
        nixpkgs.lib.genAttrs hostNames (name: mkHost [ (./hosts + "/${name}") ])
        # An installer image per host, carrying that host's whole built
        # system. Derived rather than listed: a machine added to hosts/ gets
        # one without anybody remembering to write it down, which is the same
        # reason the hosts themselves are read from the directory.
        //
        nixpkgs.lib.genAttrs (map (n: "installer-${n}") hostNames) (imageName:
          let
            host = nixpkgs.lib.removePrefix "installer-" imageName;
            target = self.nixosConfigurations.${host};
          in
          self.lib.installerFor target)
        // {
          installer-x86_64 = mkInstaller { system = "x86_64-linux"; };
          installer-aarch64 = mkInstaller { system = "aarch64-linux"; };

          # Same image plus the harness key, so the installer matrix can be
          # run against the media people actually boot.
          installer-aarch64-test =
            mkInstaller { system = "aarch64-linux"; testKey = true; };
        };
    };
}
