# Hyprland session. Shared by the VM and (later) real hardware.
{ config, pkgs, lib, ... }:

let
  # One definition, used by the greeter and by the autologin path, so they
  # cannot start the session two different ways.
  session = "${pkgs.uwsm}/bin/uwsm start -F -- /run/current-system/sw/bin/Hyprland";

  # The palette the machine was built with. The greeter runs before any user
  # session exists, so it cannot follow `kiwami theme set` - it follows the
  # theme in the flake, which is the only one that exists at that point.
  palette = config.kiwami.theme.themes.${config.kiwami.theme.name};

  # The wallpaper behind the greeter.
  #
  # Kiwami's shipped default rather than the user's own: the greeter runs as
  # the `greeter` account, which has no business reading anybody's home
  # directory - and on an ephemeral root that directory is not mounted yet
  # either. Rasterised at build time because GTK wants a picture.
  greeterBackground = pkgs.runCommand "kiwami-greeter-bg.png" { } ''
    ${pkgs.librsvg}/bin/rsvg-convert -w 2560 -h 1600 \
      ${../config/wallpaper/default.svg} -o $out
  '';
in

{
  programs.hyprland = {
    enable = true;
    # UWSM wraps the compositor in a real systemd user session: it imports the
    # environment, then activates graphical-session-pre -> graphical-session ->
    # xdg-desktop-autostart, and unwinds them on logout. Without it
    # graphical-session.target never activates and any user unit bound to it
    # silently never starts.
    withUWSM = true;
  };

  # withUWSM only flips programs.uwsm.enable; the compositor still has to be
  # registered so UWSM knows what to launch.
  programs.uwsm.waylandCompositors.hyprland = {
    prettyName = "Hyprland";
    comment = "Hyprland, managed by UWSM";
    binPath = "/run/current-system/sw/bin/Hyprland";
  };

  # Autologin straight into Hyprland: the VM must reach a desktop with no
  # interaction so the agent harness can screenshot it.
  #
  # Launched through UWSM rather than the Hyprland binary (or start-hyprland),
  # so the session gets its systemd targets. This mirrors the Exec= line the
  # uwsm module writes into its own wayland-session desktop entry.
  # Launched through UWSM rather than start-hyprland, so the session gets its
  # systemd targets. This is the same Exec line nixpkgs' own
  # hyprland-uwsm.desktop carries; Hyprland warns that it was not started via
  # start-hyprland, which is expected on this path - start-hyprland does its
  # own session setup and would fight UWSM for it.
  # ReGreet: the greeter as a window rather than a console.
  #
  # It sets services.greetd.settings.default_session itself, which is why the
  # tuigreet block above is conditional - two modules writing that one setting
  # is a conflict, not a fallback.
  #
  # Deliberately not a Quickshell greeter, for now. greetd's protocol is JSON
  # behind a binary length prefix, which is miserable to parse in QML, and a
  # greeter that crashes is a machine nobody can log into. ReGreet is somebody
  # else's tested code doing the part where being wrong locks you out.
  programs.regreet = lib.mkIf (config.kiwami.greeter == "graphical") {
    enable = true;

    settings = {
      background = {
        path = greeterBackground;
        fit = "Cover";
      };
      GTK.application_prefer_dark_theme = true;
      appearance.greeting_msg = config.kiwami.greeting;
    };

    # The palette, applied to somebody else's widgets.
    #
    # Generated from kiwami.theme rather than written out, so the greeter
    # follows the theme the machine was built with instead of being a second
    # place colours are decided. It cannot follow `kiwami theme set` - that
    # switches at runtime and this is chosen before any user session exists.
    extraCss = ''
      window {
        background-color: ${palette.background};
        color: ${palette.foreground};
      }
      .background { background-color: transparent; }
      box.horizontal > button, entry, .linked > button {
        background-color: ${palette.surface};
        color: ${palette.foreground};
        border: 1px solid ${palette.lighterBackground};
        border-radius: 8px;
      }
      entry:focus, button:focus {
        border-color: ${palette.accent};
        box-shadow: none;
      }
      button:hover { background-color: ${palette.lighterBackground}; }
      button.suggested-action {
        background-color: ${palette.accent};
        color: ${palette.darkBackground};
        border-color: ${palette.accent};
      }
      label { color: ${palette.foreground}; }
      /* The clock and greeting, which should recede rather than shout. */
      label.title-1, label.title-2 { color: ${palette.lightForeground}; }
    '';
  };

  services.greetd = {
    enable = true;
    settings = {
      # The greeter. Asks who you are, then starts the session as them.
      default_session = lib.mkIf (config.kiwami.greeter == "tui") {
        # --user-menu because a login prompt that wants a username typed from
        # memory is a login prompt that assumes you know what the accounts on
        # this machine are called. They are declared in the flake; the greeter
        # can just list them.
        #
        # --asterisks so a password that is not being received looks different
        # from one that is - on a machine that boots straight to this, silence
        # while typing reads as a wedged greeter.
        command = lib.concatStringsSep " " [
          "${pkgs.tuigreet}/bin/tuigreet"
          "--time"
          "--remember"
          "--user-menu"
          "--asterisks"
          "--greeting '${config.kiwami.greeting}'"
          "--cmd '${session}'"
        ];
        user = "greeter";
      };
    }
    # Only when explicitly asked for: see kiwami.autoLogin.
    // lib.optionalAttrs config.kiwami.autoLogin {
      initial_session = {
        command = session;
        user = config.kiwami.user;
      };
    };
  };

  environment.systemPackages = with pkgs; [
    ghostty
    hyprlock          # the locker; the shell must never draw its own
    wireplumber       # wpctl, for the volume binds
    brightnessctl     # no backlight in the VM, present for real hardware
    libnotify         # notify-send, and what most apps link against
    wl-clipboard
    grim
    slurp
    # Synthetic input, so the desktop can be driven by something other than
    # fingers: pressing SUPER+SPACE and checking a launcher actually appeared
    # is a test, watching the process exist is not.
    #
    # Both are plain Wayland clients using the virtual-keyboard and
    # virtual-pointer protocols, so they need session access and nothing more.
    # ydotool would cover both from one binary, but only by way of a root
    # daemon holding /dev/uinput open - a much larger surface for the same
    # result.
    wtype             # keyboard
    wlrctl            # pointer, and window queries
  ];

  fonts.packages = with pkgs; [
    nerd-fonts.jetbrains-mono
    noto-fonts
  ];

  # Battery and power-profile data for the bar. No battery in the VM, so the
  # widget stays hidden there; this is for real hardware.
  services.upower.enable = true;

  # Themes ship with the system so an installed machine has them without a
  # checkout. `kiwami` still prefers ~/kiwami/config/themes when it exists, so
  # editing a theme live keeps working.
  # /etc content is generated from options; see modules/generated.nix.

  security.polkit.enable = true;
  services.dbus.enable = true;
  xdg.portal = {
    enable = true;
    extraPortals = [ pkgs.xdg-desktop-portal-gtk ];
  };
}
