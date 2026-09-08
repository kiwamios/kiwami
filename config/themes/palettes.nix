# The palettes Kiwami ships, as plain data.
#
# Data rather than a NixOS module because two very different things need it:
# the desktop, which reads it through kiwami.theme, and the installer image,
# which imports none of Kiwami's modules - it has no business carrying a
# Hyprland desktop - but still wants the console to look like the same system.
#
# One definition, so the console and the shell cannot drift apart.
{
    kiwami = {
      accent = "#7ad07a";  accentDim = "#4f8f56";
      selection = "#2a3830";  muted = "#4b5a52";
      background = "#0f1411";  darkBackground = "#0a0e0c";
      lighterBackground = "#1a221d";  surface = "#151d18";
      foreground = "#d6e0d8";  darkForeground = "#6d7a72";
      lightForeground = "#b9c6bd";
      red = "#e06c75";  orange = "#d99a5e";  yellow = "#d9c46a";
      green = "#7ad07a";  cyan = "#63c9c1";  blue = "#6aa9d9";
      magenta = "#c08ad0";
      brightRed = "#f08a92";  brightOrange = "#f0b478";
      brightYellow = "#f0dd8a";  brightGreen = "#96e096";
      brightCyan = "#82e0d8";  brightBlue = "#8ac2f0";
      brightMagenta = "#d6a6e6";
    };
    midnight = {
      accent = "#8a7ae0";  accentDim = "#5b4f9e";
      selection = "#2c2a46";  muted = "#4d4a66";
      background = "#111019";  darkBackground = "#0c0b12";
      lighterBackground = "#1c1a28";  surface = "#171522";
      foreground = "#d8d6e0";  darkForeground = "#726f85";
      lightForeground = "#bab7c6";
      red = "#e06c8a";  orange = "#d99a6c";  yellow = "#d9c96a";
      green = "#7ad0a0";  cyan = "#63c1c9";  blue = "#6a9ad9";
      magenta = "#c08ad0";
      brightRed = "#f08aa4";  brightOrange = "#f0b488";
      brightYellow = "#f0e08a";  brightGreen = "#96e0b8";
      brightCyan = "#82d8e0";  brightBlue = "#8ab6f0";
      brightMagenta = "#d6a6e6";
  };
}
