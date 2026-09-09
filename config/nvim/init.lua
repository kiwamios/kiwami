-- Kiwami's editor.
--
-- Derived from kickstart.nvim, with its two package managers removed. lazy
-- fetched plugins at runtime and mason downloaded language servers; on this
-- distro nix supplies both, from the store, before nvim ever starts. What is
-- left here is only configuration - no bootstrap, no clone, no build step,
-- and nothing that needs writing to a directory nvim cannot write to.
--
-- The plugins this file configures are listed in modules/neovim.nix. Adding
-- one means adding it there and configuring it here, in that order; there is
-- no :Lazy to run and no lockfile to update.
--
-- The mac keeps the lazy-based version at github:jimzer/kickstart.nvim. The
-- two have the same keymaps on purpose, and are otherwise separate.

-- Leader has to be set before anything that defines a mapping.
vim.g.mapleader = ' '
vim.g.maplocalleader = ' '

-- No Nerd Font in the terminal, so nothing here asks for glyphs that would
-- render as boxes. Setting this true means also adding nvim-web-devicons to
-- the plugin list in modules/neovim.nix.
vim.g.have_nerd_font = false

-- [[ Options ]]

vim.opt.number = true
vim.opt.mouse = 'a'
vim.opt.showmode = false -- the statusline already says it

-- Scheduled after startup because reading the system clipboard is slow enough
-- to be noticeable at launch.
vim.schedule(function()
  vim.opt.clipboard = 'unnamedplus'
end)

vim.opt.breakindent = true
vim.opt.undofile = true

-- Case-insensitive search unless the pattern contains a capital or \C.
vim.opt.ignorecase = true
vim.opt.smartcase = true

vim.opt.signcolumn = 'yes' -- always, so the text does not jump when a sign appears
vim.opt.updatetime = 250
vim.opt.timeoutlen = 300
vim.opt.splitright = true
vim.opt.splitbelow = true

-- Show the whitespace that usually causes the argument.
vim.opt.list = true
vim.opt.listchars = { tab = '» ', trail = '·', nbsp = '␣' }

vim.opt.inccommand = 'split' -- preview :s as it is typed
vim.opt.cursorline = true
vim.opt.scrolloff = 10

-- [[ Keymaps ]]

vim.keymap.set('n', '<Esc>', '<cmd>nohlsearch<CR>')
vim.keymap.set('n', '<leader>q', vim.diagnostic.setloclist, { desc = 'Open diagnostic [Q]uickfix list' })

-- <C-\><C-n> is the real way out of terminal mode and nobody guesses it.
vim.keymap.set('t', '<Esc><Esc>', '<C-\\><C-n>', { desc = 'Exit terminal mode' })

vim.keymap.set('n', '<C-h>', '<C-w><C-h>', { desc = 'Move focus to the left window' })
vim.keymap.set('n', '<C-l>', '<C-w><C-l>', { desc = 'Move focus to the right window' })
vim.keymap.set('n', '<C-j>', '<C-w><C-j>', { desc = 'Move focus to the lower window' })
vim.keymap.set('n', '<C-k>', '<C-w><C-k>', { desc = 'Move focus to the upper window' })

-- [[ Autocommands ]]

vim.api.nvim_create_autocmd('TextYankPost', {
  desc = 'Highlight when yanking (copying) text',
  group = vim.api.nvim_create_augroup('kiwami-highlight-yank', { clear = true }),
  callback = function()
    vim.highlight.on_yank()
  end,
})

local indent_group = vim.api.nvim_create_augroup('kiwami-filetype-indent', { clear = true })

vim.api.nvim_create_autocmd('FileType', {
  pattern = { 'python', 'lua' },
  callback = function()
    vim.opt_local.tabstop = 8
    vim.opt_local.shiftwidth = 0
    vim.opt_local.expandtab = false
  end,
  group = indent_group,
})

vim.api.nvim_create_autocmd('FileType', {
  pattern = { 'javascript', 'typescript', 'javascriptreact', 'typescriptreact' },
  callback = function()
    vim.opt_local.tabstop = 4
    vim.opt_local.shiftwidth = 4
    vim.opt_local.expandtab = true
  end,
  group = indent_group,
})

-- [[ Plugins ]]
--
-- Every plugin is already on the runtimepath, so these are plain requires in
-- load order rather than a dependency graph. The colourscheme goes first so
-- nothing paints twice.

require 'kiwami.ui'
require 'kiwami.treesitter'
require 'kiwami.telescope'
require 'kiwami.lsp'
require 'kiwami.completion'
require 'kiwami.format'
require 'kiwami.git'
