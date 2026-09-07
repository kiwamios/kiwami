# Turns kiwami.* options into the files the runtime consumes.
#
# The runtime contract is unchanged on purpose: themes are still colors.json,
# still watched by the shell, still switchable with `kiwami theme set` and no
# rebuild. Nix decides what ships and validates it; the runtime decides what
# is active.
{ config, lib, pkgs, ... }:

let
  cfg = config.kiwami;

  # Option names are camelCase because that is the Nix convention; the JSON
  # keys stay snake_case because the CLI and QML already read them and the
  # runtime format is a contract with downloaded themes.
  jsonKey = name:
    lib.toLower (builtins.replaceStrings
      (map (c: c) [ "A" "B" "C" "D" "E" "F" "G" "H" "I" "J" "K" "L" "M"
                    "N" "O" "P" "Q" "R" "S" "T" "U" "V" "W" "X" "Y" "Z" ])
      (map (c: "_" + c) [ "a" "b" "c" "d" "e" "f" "g" "h" "i" "j" "k" "l" "m"
                          "n" "o" "p" "q" "r" "s" "t" "u" "v" "w" "x" "y" "z" ])
      name);

  paletteJson = name: palette:
    builtins.toJSON ({ inherit name; mode = "dark"; }
      // lib.mapAttrs' (k: v: lib.nameValuePair (jsonKey k) v) palette);

  themeFiles = lib.mapAttrs' (name: palette:
    lib.nameValuePair "kiwami/themes/${name}/colors.json" {
      text = paletteJson name palette;
    }) cfg.theme.themes;

  # What the shell reads to lay itself out. Generated rather than hardcoded in
  # QML so `kiwami.bar.right = [...]` in a consumer's flake actually moves things.
  # Ghostty writes booleans as true/false and everything else bare.
  renderGhostty = key: value:
    "${key} = ${
      if lib.isBool value then (if value then "true" else "false")
      else toString value
    }";

  barManifest = builtins.toJSON {
    inherit (cfg.bar) enable position height left center right;
    defaultTheme = cfg.theme.name;
  };
in
{
  environment.etc = themeFiles // {
    "kiwami/bar.json".text = barManifest;

    # Read by hyprland.lua. A lua file rather than a flag file so the config
    # reads it the same way it reads the theme, and so the value is the value
    # rather than the presence of a path.
    "kiwami/animations.lua".text = "return ${if cfg.animations then "true" else "false"}\n";

    # The persist list, where the CLI can read it without evaluating Nix.
    # Where `kiwami passwd` writes. Generated so the CLI reads the option
    # rather than carrying its own copy of the path.
    "kiwami/password-dir".text = cfg.passwordFile;

    "kiwami/persist.json".text = builtins.toJSON {
      directories = cfg.persist.directories;
      files = cfg.persist.files;
      # Relative to the desktop user's home. Without these the report calls
      # ~/.local undeclared while ~/.local/state is being persisted.
      user = cfg.user;
      userDirectories = cfg.persist.userDirectories;
    };
    # Where this machine came from, for `kiwami update`. Written by Nix
    # because it is a fact about how the system was built, and the only place
    # that knows it is the build.
    "kiwami/origin.json".text = builtins.toJSON {
      flake = cfg.flake;
      host = if cfg.host == "" then config.networking.hostName else cfg.host;
    };

    # Where the wallpaper comes from and how it behaves. The directory is
    # resolved to an absolute path here: QML has no notion of ~.
    "kiwami/wallpaper.json".text = builtins.toJSON {
      inherit (cfg.wallpaper) enable rotate interval fit;
      directory =
        if cfg.wallpaper.directory != ""
        then cfg.wallpaper.directory
        else "/home/${cfg.user}/Pictures/wallpapers";
      # Shown when the directory is empty or missing, so a machine that has
      # just been installed has a desktop rather than a black rectangle.
      fallback = "/etc/kiwami/wallpaper-default.svg";
    };

    # SVG rather than a photograph: a few hundred bytes that scale to any
    # screen, instead of a binary blob in a git repository that everyone who
    # clones this has to download forever.
    "kiwami/wallpaper-default.svg".source = ../config/wallpaper/default.svg;

    "kiwami/shell".source = ../shell;

    # Ours, a real Ghostty file rather than generated - same treatment as the
    # Hyprland Lua, and for the same reason: it reads better in its own syntax.
  };
}
