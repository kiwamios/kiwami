# The editor, built rather than installed.
#
# Neovim ships a plugin manager, and kickstart adds two more - lazy for
# plugins and mason for language servers. All three fetch things at runtime,
# into the home directory, which on this distro means: a first launch that
# clones thirty repositories, a C compiler on every machine so treesitter can
# build its parsers, a directory that must be declared persistent or the work
# is lost at reboot, and a lockfile nvim cannot write because the config is a
# read-only store path. Four problems, one cause.
#
# So nix does that job instead. Plugins and servers come from the store,
# already built, on the runtimepath before nvim starts. Nothing is fetched,
# nothing is compiled on the machine, and there is no state to keep.
#
# The configuration in config/nvim is compiled in the same way: it lives in
# the store, on the runtimepath, and is not in ~/.config at all - so there is
# no read-only symlink to be surprised by.
#
# Changing the editor means editing this file and config/nvim together, then
# `kiwami update`. Adding a plugin: a line here and its setup call there.
{ config, lib, pkgs, ... }:

let
  cfg = config.kiwami.apps;

  # What the config in config/nvim/lua/kiwami expects to be able to require.
  #
  # nvim-web-devicons is deliberately absent: the config sets
  # have_nerd_font = false, and glyphs without the font are boxes.
  plugins = with pkgs.vimPlugins; [
    # Editing
    vim-sleuth
    mini-nvim
    nvim-autopairs
    indent-blankline-nvim
    todo-comments-nvim

    # Look
    dracula-nvim
    which-key-nvim

    # Finding things
    telescope-nvim
    plenary-nvim
    telescope-fzf-native-nvim
    telescope-ui-select-nvim

    # The tree, and what it is built from
    neo-tree-nvim
    nui-nvim

    # Language servers
    nvim-lspconfig
    lazydev-nvim
    fidget-nvim

    # Completion
    nvim-cmp
    cmp-nvim-lsp
    cmp-path
    cmp_luasnip
    luasnip

    # Formatting and linting
    conform-nvim
    nvim-lint

    # Git
    gitsigns-nvim
    diffview-nvim

    # Every grammar, pre-built. This is the entry that removes gcc, gnumake
    # and tree-sitter from the machine: nothing is compiled at runtime, so
    # nothing needs a toolchain to compile it with.
    nvim-treesitter.withAllGrammars
  ];

  # The binaries the config shells out to.
  #
  # Wrapped onto nvim's PATH rather than installed system-wide, so the editor
  # always finds its tools and a machine does not grow six commands nobody
  # asked for. The names here have to match config/nvim/lua/kiwami/lsp.lua and
  # format.lua - a mismatch is silent, and shows up as a language server that
  # simply never attaches.
  tools = with pkgs; [
    # Language servers
    lua-language-server # lua_ls
    pyright # pyright
    typescript-language-server # ts_ls

    # Formatters and linters
    stylua # lua
    ruff # python, both
    biome # js/ts, both
    markdownlint-cli # markdown

    # Telescope's live_grep and find_files run these.
    ripgrep
    fd
  ];

  # The config directory as a store path, prepended to the runtimepath so
  # `require 'kiwami.lsp'` resolves to config/nvim/lua/kiwami/lsp.lua.
  configDir = ../config/nvim;

  neovim = pkgs.neovim.override {
    withNodeJs = false;
    withPython3 = false;
    withRuby = false;

    configure = {
      customRC = ''
        set runtimepath^=${configDir}
        luafile ${configDir}/init.lua
      '';
      packages.kiwami.start = plugins;
    };

    extraMakeWrapperArgs = "--suffix PATH : ${lib.makeBinPath tools}";
  };
in
{
  config = lib.mkIf cfg.terminal {
    environment.systemPackages = [ neovim ];

    # Set here rather than in a shell profile so it holds for anything that
    # spawns an editor - git, systemctl edit, a script that reads $EDITOR.
    #
    # mkOverride 900 rather than mkDefault: NixOS itself defaults this to
    # nano, and two defaults at the same priority are a conflict rather than a
    # winner. 900 beats a default and still loses to a host that states its
    # own preference outright.
    environment.variables.EDITOR = lib.mkOverride 900 "nvim";
  };
}
