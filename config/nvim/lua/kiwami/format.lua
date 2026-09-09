-- Formatting on save, and linting as you go.
--
-- Every tool named here is installed by modules/neovim.nix and found on PATH.
-- Kickstart left that to mason; the list below and the package list there have
-- to agree, and a name in one and not the other is the way this quietly stops
-- working.

require('conform').setup {
  notify_on_error = false,
  format_on_save = function(bufnr)
    -- C and C++ have no settled house style, so formatting them on save
    -- produces diffs nobody asked for.
    local disable_filetypes = { c = true, cpp = true }
    local lsp_format_opt
    if disable_filetypes[vim.bo[bufnr].filetype] then
      lsp_format_opt = 'never'
    else
      lsp_format_opt = 'fallback'
    end
    return {
      timeout_ms = 500,
      lsp_format = lsp_format_opt,
    }
  end,
  formatters_by_ft = {
    lua = { 'stylua' },
    python = { 'ruff_fix', 'ruff_format', 'ruff_organize_imports' },
    javascript = { 'biome', 'biome-check', 'biome-organize-imports' },
    typescript = { 'biome', 'biome-check', 'biome-organize-imports' },
    javascriptreact = { 'biome', 'biome-check', 'biome-organize-imports' },
    typescriptreact = { 'biome', 'biome-check', 'biome-organize-imports' },
  },
}

vim.keymap.set('', '<leader>f', function()
  require('conform').format { async = true, lsp_format = 'fallback' }
end, { desc = '[F]ormat buffer' })

local lint = require 'lint'
lint.linters_by_ft = {
  markdown = { 'markdownlint' },
  javascript = { 'biomejs' },
  typescript = { 'biomejs' },
  javascriptreact = { 'biomejs' },
  typescriptreact = { 'biomejs' },
  python = { 'ruff' },
}

local lint_augroup = vim.api.nvim_create_augroup('kiwami-lint', { clear = true })
vim.api.nvim_create_autocmd({ 'BufEnter', 'BufWritePost', 'InsertLeave' }, {
  group = lint_augroup,
  callback = function()
    if not vim.opt_local.modifiable:get() then
      return
    end
    -- Only run the linters that are actually installed. Without this a
    -- missing binary produces an error on every keystroke out of insert mode.
    local runnable = {}
    for _, name in ipairs(lint.linters_by_ft[vim.bo.filetype] or {}) do
      local linter = lint.linters[name]
      local cmd = type(linter) == 'table' and linter.cmd or nil
      if type(cmd) == 'function' then
        cmd = cmd()
      end
      if cmd and vim.fn.executable(cmd) == 1 then
        table.insert(runnable, name)
      end
    end
    if #runnable > 0 then
      lint.try_lint(runnable)
    end
  end,
})
