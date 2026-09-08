# Palettes that ship with Kiwami.
#
# mkDefault throughout: a consumer can replace a whole theme, or override one
# colour of one theme, from their own flake.
{ lib, ... }:
{
  kiwami.theme.themes = lib.mkDefault (import ../config/themes/palettes.nix);
}
