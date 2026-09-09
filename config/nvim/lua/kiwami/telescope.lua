-- Fuzzy finding: files, grep, help, and most of the LSP navigation.

require('telescope').setup {
  extensions = {
    ['ui-select'] = {
      require('telescope.themes').get_dropdown(),
    },
  },
}

-- fzf-native is a compiled C sorter. Under nix it is built in the store
-- rather than by a `make` at install time, so it is simply present - but the
-- pcall stays, because a load failure should cost sorting speed and not the
-- whole editor.
pcall(require('telescope').load_extension, 'fzf')
pcall(require('telescope').load_extension, 'ui-select')

local builtin = require 'telescope.builtin'
vim.keymap.set('n', '<leader>sh', builtin.help_tags, { desc = '[S]earch [H]elp' })
vim.keymap.set('n', '<leader>sk', builtin.keymaps, { desc = '[S]earch [K]eymaps' })
vim.keymap.set('n', '<leader>sf', builtin.find_files, { desc = '[S]earch [F]iles' })
vim.keymap.set('n', '<leader>ss', builtin.builtin, { desc = '[S]earch [S]elect Telescope' })
vim.keymap.set('n', '<leader>sw', builtin.grep_string, { desc = '[S]earch current [W]ord' })
vim.keymap.set('n', '<leader>sg', builtin.live_grep, { desc = '[S]earch by [G]rep' })
vim.keymap.set('n', '<leader>sd', builtin.diagnostics, { desc = '[S]earch [D]iagnostics' })
vim.keymap.set('n', '<leader>sr', builtin.resume, { desc = '[S]earch [R]esume' })
vim.keymap.set('n', '<leader>s.', builtin.oldfiles, { desc = '[S]earch Recent Files ("." for repeat)' })
vim.keymap.set('n', '<leader><leader>', builtin.buffers, { desc = '[ ] Find existing buffers' })

vim.keymap.set('n', '<leader>/', function()
  builtin.current_buffer_fuzzy_find(require('telescope.themes').get_dropdown {
    winblend = 10,
    previewer = false,
  })
end, { desc = '[/] Fuzzily search in current buffer' })

vim.keymap.set('n', '<leader>s/', function()
  builtin.live_grep {
    grep_open_files = true,
    prompt_title = 'Live Grep in Open Files',
  }
end, { desc = '[S]earch [/] in Open Files' })

-- Kickstart pointed this at stdpath('config'). Here the config is in the nix
-- store, read-only, and searching it would find files you cannot edit - so it
-- opens the flake's checkout if there is one, and says so if there is not.
vim.keymap.set('n', '<leader>sn', function()
  local repo = vim.fn.expand '~/Projects/kiwami'
  if vim.fn.isdirectory(repo) == 0 then
    vim.notify('No kiwami checkout at ' .. repo .. ' - the running config lives in the nix store.', vim.log.levels.WARN)
    return
  end
  builtin.find_files { cwd = repo .. '/config/nvim' }
end, { desc = '[S]earch [N]eovim files' })
