# The Kiwami option surface.
#
# This is the distro's API. Everything here uses mkDefault where it is a
# preference rather than a requirement, so a consumer importing
# nixosModules.default overrides it by simply setting the option - no forking,
# and our defaults keep improving underneath what they did not touch.
{ lib, ... }:

let
  inherit (lib) mkOption mkEnableOption types;

  # A colour is #rrggbb. Catching this at eval is not pedantry: an earlier
  # theme shipped "#4d4a६6" - valid JSON, invalid colour, silently rendered
  # as garbage.
  hexColor = types.strMatching "#[0-9a-fA-F]{6}";

  paletteType = types.submodule {
    options = lib.genAttrs [
      "accent" "accentDim" "selection" "muted"
      "background" "darkBackground" "lighterBackground" "surface"
      "foreground" "darkForeground" "lightForeground"
      "red" "orange" "yellow" "green" "cyan" "blue" "magenta"
      "brightRed" "brightOrange" "brightYellow" "brightGreen"
      "brightCyan" "brightBlue" "brightMagenta"
    ] (name: mkOption {
      type = hexColor;
      description = "The ${name} colour.";
    });
  };
in
{
  options.kiwami = {
    theme = {
      name = mkOption {
        type = types.str;
        default = "kiwami";
        description = ''
          Theme applied on a machine that has never had one set. Switching
          later is a runtime operation (`kiwami theme set`), not a rebuild,
          so this is only the starting point.
        '';
      };

      themes = mkOption {
        type = types.attrsOf paletteType;
        default = { };
        description = ''
          Palettes to ship. Every one is type checked, so a theme cannot be
          missing a key or carry a malformed colour. Themes downloaded at
          runtime land in ~/.config/kiwami/themes as plain JSON instead -
          deliberately, since a Nix theme would be arbitrary code.
        '';
      };
    };

    flake = mkOption {
      type = types.str;
      default = "";
      example = "github:alice/dotfiles";
      description = ''
        The flake this machine rebuilds itself from. `kiwami update` builds
        `<flake>#nixosConfigurations.<host>` and switches to it.

        The machine deliberately keeps no checkout, so this is how it knows
        where its own configuration lives. It was a constant in the CLI
        pointing at the author's repository, which worked for exactly one
        person: anyone else's machine would have updated itself into somebody
        else's configuration, or more likely failed to find a host by its
        name and stopped.

        Left empty, `kiwami update` explains that it has nowhere to build
        from rather than guessing.
      '';
    };

    host = mkOption {
      type = types.str;
      default = "";
      example = "thinkpad";
      description = ''
        Which attribute of `kiwami.flake` describes this machine. Empty means
        `networking.hostName`, which is nearly always right - set it only when
        the flake calls the machine something other than what the machine
        calls itself.
      '';
    };

    wallpaper = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Draw a wallpaper on the background layer of every screen.";
      };

      directory = mkOption {
        type = types.str;
        default = "";
        example = "/home/alice/wallpapers";
        description = ''
          Where the images live. Empty means
          ~/.local/share/kiwami/wallpapers for kiwami.user.

          Deliberately outside the flake. Wallpapers are pictures - you add
          one, you look through them to pick one. Putting them in the store
          would mean a rebuild to change a picture, and a git repository that
          grows by megabytes a time.

          Also deliberately not ~/Pictures. That directory is load-bearing
          for other things - a photo library, a screenshot folder - and it
          has to be persisted for wallpapers to survive a boot, which would
          drag an entire photo library into /persist and therefore into
          every backup. A directory Kiwami owns can be persisted without
          deciding anything about the rest of your pictures.

          Whatever you set has to be persisted, or an ephemeral root deletes
          it overnight. The default is.
        '';
      };

      rotate = mkOption {
        type = types.bool;
        default = false;
        description = ''
          Cycle through the images. With one image, or with this off, the
          first image by filename is shown and nothing changes.

          Sorted by filename rather than taken in directory order, which
          varies by filesystem: "first" has to mean the same thing on your
          laptop as it does in a test VM.
        '';
      };

      interval = mkOption {
        type = types.ints.positive;
        default = 900;
        description = "Seconds between images, when rotating.";
      };

      fit = mkOption {
        type = types.enum [ "cover" "contain" "fill" "tile" ];
        default = "cover";
        description = ''
          How an image fills a screen it does not match. `cover` crops to
          fill, `contain` fits the whole image and leaves bars, `fill`
          stretches, `tile` repeats.
        '';
      };
    };

    greeter = mkOption {
      type = types.enum [ "graphical" "tui" ];
      default = "graphical";
      description = ''
        Which greeter asks who you are.

        `graphical` is ReGreet: a real window with the accounts listed, the
        wallpaper behind it, and Kiwami's palette. `tui` is tuigreet, a text
        prompt on a console - smaller, and the thing to fall back to on a
        machine whose graphics are not cooperating.

        Neither is used when kiwami.autoLogin is on, since nothing is asked.
      '';
    };

    greeting = mkOption {
      type = types.str;
      default = "kiwami";
      description = "The line the greeter shows above the prompt.";
    };

    splash = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Replace the boot log with a splash, and draw the disk passphrase
          prompt inside it rather than on a bare console.

          Presentation only. Turned off you get the kernel and systemd output
          back, which is what you want the morning something fails before the
          desktop exists - so it is a switch rather than something to comment
          out.
        '';
      };

      theme = mkOption {
        type = types.str;
        default = "spinner";
        description = ''
          A Plymouth theme name. The default draws Kiwami's mark with a
          spinner beneath it; set kiwami.splash.logo to change the mark
          without leaving the theme.
        '';
      };
    };

    user = mkOption {
      type = types.strMatching "[a-z_][a-z0-9_-]*";
      default = "nixos";
      description = ''
        The account the desktop belongs to: the one greetd logs in, and the
        one whose home carries the Hyprland and Quickshell configuration.

        This was hardcoded, and the effect of getting it wrong is quiet. A
        machine installed for a differently-named user came up on Hyprland's
        own autogenerated config with no bar, because greetd logged in an
        account that had no home-manager configuration attached - everything
        was installed, just for somebody else.
      '';
    };

    ephemeralRoot = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Wipe the root filesystem at every boot, keeping only what
        kiwami.persist declares.

        On by default: every machine `kiwami install` creates is ephemeral,
        and the installer does not offer the alternative. Two layouts would
        mean two sets of assumptions about where state lives and a class of
        bug that only shows up on one of them.

        It remains an option only because three harness hosts predate it and
        still have a single ext4 root. They set it false explicitly; converting
        them is follow-up work, not a supported mode.

        Requires a layout with @root, @nix and @persist subvolumes and a
        @root-blank snapshot to restore from, so it cannot simply be switched
        on for a machine installed without one.
      '';
    };

    rootDevice = mkOption {
      type = types.str;
      default = "/dev/disk/by-partlabel/disk-system-root";
      description = ''
        The block device holding the btrfs filesystem whose subvolumes make up
        the root.

        The rollback runs in the initrd and has to mount the filesystem's top
        level before anything else has it, so it needs the device by name -
        and on an encrypted machine that name is not the partition. It is
        /dev/mapper/<name>, which only exists once the container is unlocked.

        Encryption and the ephemeral root were built separately and never ran
        together: the rollback named the partition unconditionally, so on an
        encrypted machine it would have tried to mount a LUKS container as
        btrfs, failed in the initrd, and dropped the machine into emergency
        mode on the first boot after installing. Which is the most expensive
        moment to find out.
      '';
      example = "/dev/mapper/cryptroot";
    };

    passwordFile = mkOption {
      type = types.str;
      default = "/var/lib/kiwami/passwords";
      description = ''
        Directory holding one hashed password per user, read at activation.

        The hash lives on the machine and never in the repository, so the
        config stays publishable while the secret does not travel.

        Under /var/lib/kiwami deliberately: that path is already persisted, so
        the same arrangement works whether or not the root is ephemeral. There
        is no second mode to reason about.

        users.mutableUsers is off everywhere as a result, which is what makes
        this the only way in - nixpkgs drops the setuid passwd wrapper when
        users are immutable, so the wrong command is not merely discouraged,
        it is absent.
      '';
    };

    backup = {
      enable = mkOption {
        type = types.bool;
        default = false;
        description = ''
          Back /persist up to an S3-compatible bucket with restic.

          /persist is the whole target, deliberately. The machine is
          declarative, so the only irreplaceable state is what
          kiwami.persist declares - which makes one list serve three
          purposes: what survives a reboot, what survives the disk dying,
          and what `kiwami doctor` checks for gaps. There are no
          per-directory backup policies to drift out of date.
        '';
      };

      repository = mkOption {
        type = types.str;
        default = "";
        description = ''
          The restic repository, including any path prefix.

          A prefix is what lets one bucket hold more than this: point two
          machines at s3:.../backups/kiwami-xps and .../kiwami-laptop and they
          are separate repositories with separate passwords, sharing nothing.
          That is the safer default - restic has no per-host permissions
          inside a repository, so a compromised machine could delete another's
          snapshots.

          Not a secret, which is why it lives here rather than in the
          credentials file. It names where the backups are, not how to open
          them.
        '';
        example = "s3:https://ACCOUNT.r2.cloudflarestorage.com/backups/kiwami-xps";
      };

      credentialsFile = mkOption {
        type = types.str;
        default = "/var/lib/kiwami/backup/env";
        description = ''
          Where the repository password and the bucket's access keys live, as
          an environment file readable only by root.

          Under /var/lib/kiwami because that path is already persisted, so the
          arrangement is the same whether or not the root is ephemeral.

          Encrypting this on disk would buy very little: anything that can
          decrypt it at runtime is exactly what an attacker would be running
          as. Encryption at rest is what the disk is for. What does help is
          immutability at the far end - a bucket lock means a compromised
          machine cannot destroy the backups it can write.
        '';
      };

      exclude = mkOption {
        type = types.listOf types.str;
        default = [
          "node_modules"
          ".venv"
          "__pycache__"
          ".direnv"
          "dist"
          "build"
          ".next"
          "target"
          "result"
          "result-*"
        ];
        description = ''
          Directory names never worth storing: build output, dependency
          trees, and Nix result symlinks.

          Source is backed up wholesale rather than cleverly. It is tempting
          to skip anything already committed and pushed, but the valuable
          part of a working tree is exactly what is not - uncommitted work,
          stashes, .env files, scratch notes. Those are small. Dropping build
          artifacts gets nearly all of the size win with none of that risk.

          restic also honours CACHEDIR.TAG, which Cargo writes into target/,
          so Rust build output disappears without being named. It is listed
          anyway: belt and braces, and the list should read as what it means.
        '';
      };

      schedule = mkOption {
        type = types.str;
        default = "daily";
        description = ''
          A systemd calendar expression. Daily suits a machine whose state
          changes slowly; the backup is incremental, so a run with nothing
          new costs a handful of requests.
        '';
      };

      keep = mkOption {
        type = types.attrsOf types.int;
        default = { daily = 7; weekly = 4; monthly = 6; };
        description = ''
          What `restic forget` keeps. Pruning is the expensive operation
          against object storage, so it is monthly rather than per-run.
        '';
      };
    };

    persist = {
      directories = mkOption {
        type = types.listOf types.str;
        default = [ ];
        description = ''
          Directories that must survive a wipe of the root filesystem.

          Nothing reads this to wipe anything yet. It exists so the question
          "what would I lose" has an answer that can be checked: `kiwami
          doctor` compares it against the state actually on the machine and
          reports what is not covered. The list gets built from evidence
          rather than from guesswork, which matters because the entries people
          forget are the ones nothing complains about until much later.
        '';
        example = [ "/var/lib/tailscale" "/var/lib/nixos" ];
      };

      files = mkOption {
        type = types.listOf types.str;
        default = [ ];
        description = "Individual files that must survive, e.g. host keys.";
        example = [ "/etc/machine-id" ];
      };

      userFiles = mkOption {
        type = types.listOf types.str;
        default = [ ];
        description = ''
          Individual files under the desktop user's home that must survive,
          relative to it.

          The reason this exists alongside userDirectories: a directory is the
          wrong unit when it mixes a secret with declarative config.
          ~/.config/gh holds hosts.yml (your token, must survive, cannot come
          from the flake), config.yml (preferences, comes from the flake) and
          state.yml (an update-check cache that should survive nothing).
          Persisting the directory takes all three, and then home-manager and
          the bind mount both own config.yml - the flake writes a store symlink
          into /persist, which dangles once that generation is collected. It
          half-works, which is worse than failing.

          So: no path gets two owners. Persist the file, let the flake own the
          rest of the directory.
        '';
        example = [ ".config/gh/hosts.yml" ];
      };

      userDirectories = mkOption {
        type = types.listOf types.str;
        # The real list lives in modules/common.nix. A default here would be
        # silently discarded as soon as anything defined a value - which is
        # exactly what happened, dropping ~/.ssh and ~/kiwami without a word.
        default = [ ];
        description = ''
          Paths under the desktop user's home that must survive, relative to
          it. They are stored in /persist/home/<user> and bound back.

          The defaults are not decoration:

          ~/kiwami is the flake the machine rebuilds itself from. A machine
          that wipes its own configuration on first reboot is one you can no
          longer change.

          ~/.local/state holds the selected theme, among other runtime state
          the desktop keeps between sessions. Without it the theme resets at
          every boot and `kiwami doctor` reports "no theme applied" on a
          machine that was themed yesterday.

          ~/Projects is work in progress. It is mostly in git and mostly
          pushed, but the part that matters is the part that is not:
          uncommitted changes, stashes, .env files, scratch notes. That is
          also what makes it the one entry here whose size is unbounded, which
          is why `kiwami snapshot` excludes build output rather than trying to
          be clever about what git already has.
        '';
      };
    };

    animations = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Hyprland's window animations.

        On, because this is a desktop and it should feel like one. Off only
        where there is no GPU to run them: a VM on llvmpipe pays for every
        frame in software, and animations also make screenshots
        nondeterministic, which is how the test VM ended up defining the
        default for real hardware.

        Read by config/hypr/hyprland.lua from a generated file, so changing it
        is a rebuild rather than an edit to a config the flake owns.
      '';
    };

    autoLogin = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Log the desktop user straight in, with no password.

        Off by default. It is right for a throwaway VM that exists to be
        driven by a harness, and wrong for a laptop: it means anyone who opens
        the lid is you. Combined with an unencrypted disk it means the machine
        has no access control at all, which is easy not to notice because
        nothing about a working desktop looks wrong.
      '';
    };

    bar = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Whether to run the top bar at all.";
      };

      position = mkOption {
        type = types.enum [ "top" "bottom" ];
        default = "top";
      };

      height = mkOption {
        type = types.ints.positive;
        default = 32;
      };

      left = mkOption {
        type = types.listOf types.str;
        default = [ "workspaces" ];
        description = ''
          Widgets on the left, in order. Names resolve to QML components,
          user-provided ones first. Lists merge across modules, so a profile
          can add a widget without replacing the list.
        '';
      };

      center = mkOption {
        type = types.listOf types.str;
        default = [ "window" ];
      };

      right = mkOption {
        type = types.listOf types.str;
        default = [ "tray" "network" "backup" "battery" "clock" ];
      };
    };

    terminal = {
      settings = mkOption {
        type = types.attrsOf (types.oneOf [ types.str types.int types.bool ]);
        default = { };
        example = { font-size = 14; cursor-style = "bar"; };
        description = ''
          Per-machine Ghostty overrides, rendered to a file included after
          Kiwami's defaults and the theme. Empty by default: the defaults are
          a real Ghostty file (config/ghostty/defaults), not generated from
          here, so this only carries what a consumer actually changes.
        '';
      };

      extraConfig = mkOption {
        # `lines` merges by concatenation, so several modules and the user can
        # all contribute. Ghostty applies the last definition of a key.
        type = types.lines;
        default = "";
        description = "Appended verbatim to the generated Ghostty config.";
      };
    };

    hyprland.extraConfig = mkOption {
      type = types.lines;
      default = "";
      description = "Lua appended after the generated Hyprland config.";
    };
  };
}
