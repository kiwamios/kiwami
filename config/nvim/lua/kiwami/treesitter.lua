-- Syntax-aware highlighting and indentation.
--
-- Two things to know, and the second is why this file looks nothing like
-- kickstart's version.
--
-- The grammars come from nix: `nvim-treesitter.withAllGrammars` in
-- modules/neovim.nix ships every one pre-compiled, on the runtimepath. There
-- is nothing to install, so no `ensure_installed`, no `auto_install`, and no
-- C compiler anywhere on the machine.
--
-- And nixpkgs ships nvim-treesitter's `main` branch - the rewrite, which
-- deleted the `nvim-treesitter.configs` module that kickstart configures.
-- That is exactly what kickstart pinned `branch = 'master'` to avoid, and the
-- pin is not available here: the version is whatever nixpkgs packaged. On
-- main the plugin no longer does highlighting at all. Neovim does, and you
-- turn it on per buffer with vim.treesitter.start().

-- Ruby's indent rules depend on vim's own regex highlighting, and treesitter
-- indent makes them worse rather than better.
local no_treesitter_indent = { ruby = true }

vim.api.nvim_create_autocmd('FileType', {
  group = vim.api.nvim_create_augroup('kiwami-treesitter', { clear = true }),
  callback = function(args)
    local ft = vim.bo[args.buf].filetype

    -- pcall because a filetype with no grammar is normal, not an error worth
    -- showing: opening a .conf should not produce a stack trace.
    if not pcall(vim.treesitter.start, args.buf) then
      return
    end

    if not no_treesitter_indent[ft] then
      vim.bo[args.buf].indentexpr = "v:lua.require'nvim-treesitter'.indentexpr()"
    end
  end,
})
