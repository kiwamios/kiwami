# The text console, made legible.
#
# A graphical installer was the obvious idea and the wrong one. The installer
# is what you reach for when a machine is broken, so it should need the least
# of that machine: no compositor, no GPU driver, and a serial line when there
# is no screen at all. Every installer test drives it exactly that way, and
# that is the property worth keeping.
#
# What was actually ugly was the font and the colours, and both are a setting.
# The stock 8x16 VGA face is why a console looks like 1994; Terminus at 24px
# is legible across a room and costs a few hundred kilobytes rather than a
# graphics stack.
#
# Deliberately standalone: the installer image imports none of Kiwami's other
# modules - it has no business carrying a desktop - so this has to work with
# or without kiwami.* being declared at all.
{ config, lib, pkgs, ... }:

let
  palettes = import ../config/themes/palettes.nix;

  # The machine's own theme when there is one, and Kiwami's default when
  # there is not. `or` rather than a conditional on some flag: on the
  # installer the option does not exist, so this is the only form that works
  # in both places.
  theme = config.kiwami.theme or null;
  palette =
    if theme == null then palettes.kiwami else theme.themes.${theme.name};

  # The console takes sixteen colours in the order terminals have used since
  # the 1980s: eight normal, then eight bright. Hex, without the hash.
  hex = c: lib.removePrefix "#" c;
in
{
  console.font = lib.mkDefault "ter-v24n";
  console.packages = [ pkgs.terminus_font ];

  console.colors = lib.mkDefault (map hex [
    palette.darkBackground   # 0  black
    palette.red              # 1  red
    palette.green            # 2  green
    palette.yellow           # 3  yellow
    palette.blue             # 4  blue
    palette.magenta          # 5  magenta
    palette.cyan             # 6  cyan
    palette.foreground       # 7  white
    palette.muted            # 8  bright black
    palette.brightRed        # 9
    palette.brightGreen      # 10
    palette.brightYellow     # 11
    palette.brightBlue       # 12
    palette.brightMagenta    # 13
    palette.brightCyan       # 14
    palette.lightForeground  # 15 bright white
  ]);
}
