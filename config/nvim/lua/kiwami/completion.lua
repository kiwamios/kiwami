-- Completion, snippets, and the bracket pairing that hangs off them.

local cmp = require 'cmp'
local luasnip = require 'luasnip'

-- Kickstart built LuaSnip with `make install_jsregexp` at install time, for
-- regex support in snippet transforms. Nix builds the plugin, so there is no
-- install step here for that to hang off.
luasnip.config.setup {}

cmp.setup {
  snippet = {
    expand = function(args)
      luasnip.lsp_expand(args.body)
    end,
  },
  completion = { completeopt = 'menu,menuone,noinsert' },

  -- Deliberately not <Tab>/<CR>: see `:help ins-completion` for why these
  -- keys and not the ones every other editor uses.
  mapping = cmp.mapping.preset.insert {
    ['<C-n>'] = cmp.mapping.select_next_item(),
    ['<C-p>'] = cmp.mapping.select_prev_item(),
    ['<C-b>'] = cmp.mapping.scroll_docs(-4),
    ['<C-f>'] = cmp.mapping.scroll_docs(4),

    -- Accept. Auto-imports if the server offers it.
    ['<C-y>'] = cmp.mapping.confirm { select = true },

    ['<C-Space>'] = cmp.mapping.complete {},

    -- Move forwards and backwards through a snippet's holes.
    ['<C-l>'] = cmp.mapping(function()
      if luasnip.expand_or_locally_jumpable() then
        luasnip.expand_or_jump()
      end
    end, { 'i', 's' }),
    ['<C-h>'] = cmp.mapping(function()
      if luasnip.locally_jumpable(-1) then
        luasnip.jump(-1)
      end
    end, { 'i', 's' }),
  },

  sources = {
    {
      name = 'lazydev',
      -- group_index 0 keeps lua_ls's own duplicate suggestions out.
      group_index = 0,
    },
    { name = 'nvim_lsp' },
    { name = 'luasnip' },
    { name = 'path' },
  },
}

-- Close the pair that a confirmed completion opened, so accepting a function
-- name gives you `name()` with the cursor inside it.
require('nvim-autopairs').setup {}
cmp.event:on('confirm_done', require('nvim-autopairs.completion.cmp').on_confirm_done())
