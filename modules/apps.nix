# The programs, and the state they keep.
#
# Package, configuration and persistence for each application live together
# here on purpose. The failure this project keeps meeting is state nobody
# declared - a program writes somewhere, the root is wiped, and the loss is
# discovered later by its absence. Splitting "install it" from "keep what it
# writes" across two files is how that happens, so they are one entry.
#
# The rule for what is persisted: whatever the *program* writes. Never what
# Nix writes - a config placed from the flake is a read-only store symlink,
# recreated at every activation, and persisting it would only shadow the real
# one with a stale copy.
{ config, lib, pkgs, inputs, ... }:

let
  cfg = config.kiwami.apps;
  user = config.kiwami.user;
in
{
  options.kiwami.apps = {
    terminal = lib.mkEnableOption "the terminal tools: neovim, zellij, herdr" // {
      default = true;
    };

    desktop = lib.mkEnableOption "the windowed applications: browser, notes, cards" // {
      default = true;
    };
  };

  config = lib.mkMerge [
    (lib.mkIf cfg.terminal {
      environment.systemPackages = [
        pkgs.neovim
        pkgs.zellij
        inputs.herdr.packages.${pkgs.stdenv.hostPlatform.system}.default
      ];

      # Nothing to persist for any of these.
      #
      # zellij keeps sessions in ~/.cache, which is meant to die with a boot,
      # and herdr keeps its state under XDG_STATE_HOME - which is
      # ~/.local/state, already persisted. neovim's shada and undo history are
      # in the same place. Verified by reading the binaries rather than
      # assumed, because "it probably uses ~/.config" is how a card collection
      # gets lost.
    })

    (lib.mkIf cfg.desktop {
      environment.systemPackages = [
        pkgs.brave
        pkgs.anki
        pkgs.obsidian
      ];

      # Brave, told what it is allowed to be.
      #
      # The module writes /etc/brave/policies/managed/, which Brave reads the
      # same way Chrome reads enterprise policy - so this is configuration
      # from the store, not something to persist.
      #
      # Shields is untouched by any of this: the ad and tracker blocker is a
      # core feature, not one of the extras below. Turning Rewards off does
      # not reduce blocking - Rewards is Brave showing you *its* ads in
      # exchange for tokens.
      programs.chromium = {
        enable = true;

        # Forced, so it survives a profile reset and arrives on a new machine
        # without anybody visiting a store page.
        extensions = [
          "nngceckbapebfimnlniiiahkandclblb" # Bitwarden
        ];

        extraOpts = {
          BraveRewardsDisabled = true;
          BraveWalletDisabled = true;
          BraveVPNDisabled = true;
          BraveAIChatEnabled = false; # Leo
          TorDisabled = true;
          MetricsReportingEnabled = false;
        };
      };

      # What these programs write, as opposed to what they are configured
      # with. Two of them hold things that cannot be rebuilt from anywhere.
      kiwami.persist.userDirectories = [
        # The card collection. Losing this loses years of review history,
        # which no amount of reinstalling brings back.
        ".local/share/Anki2"

        # Named like configuration and is not: history, bookmarks, cookies,
        # logins and extension data. Brave's actual configuration is the
        # policy file in /etc, which comes from the store.
        ".config/BraveSoftware"

        # Also state despite the name - which vaults exist, window layout.
        # Cheap to keep and mildly annoying to rebuild.
        ".config/obsidian"
      ];
    })
  ];
}
