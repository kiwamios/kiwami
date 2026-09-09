-- Language servers.
--
-- Kickstart used mason here, which downloads server binaries at runtime into
-- ~/.local/share/nvim/mason. That is a package manager inside a package
-- manager, and on NixOS a bad one: the binaries it fetches are linked against
-- a filesystem layout this machine does not have. Nix installs the servers
-- instead - see modules/neovim.nix - and nvim-lspconfig starts them by name
-- off PATH, which is all mason was arranging.
--
-- Adding a language is therefore two lines: the server package in
-- modules/neovim.nix, and an entry in `servers` below.

-- What runs, and how. The key is nvim-lspconfig's name for the server.
local servers = {
  pyright = {},
  ts_ls = {},
  lua_ls = {
    settings = {
      Lua = {
        completion = {
          callSnippet = 'Replace',
        },
      },
    },
  },
}

-- Everything below is wiring, and the same for every server.

-- Neovim's own capabilities plus the ones nvim-cmp adds, announced to each
-- server so it knows what the client can render.
local capabilities = vim.lsp.protocol.make_client_capabilities()
capabilities = vim.tbl_deep_extend('force', capabilities, require('cmp_nvim_lsp').default_capabilities())

-- lazydev teaches lua_ls about the neovim API, so editing this config gets
-- completion for `vim.*` instead of a wall of undefined-global warnings.
require('lazydev').setup {
  library = {
    { path = '${3rd}/luv/library', words = { 'vim%.uv' } },
  },
}

require('fidget').setup {}

-- The keymaps, bound per-buffer when a server attaches - so they exist in
-- files a server handles and nowhere else.
vim.api.nvim_create_autocmd('LspAttach', {
  group = vim.api.nvim_create_augroup('kiwami-lsp-attach', { clear = true }),
  callback = function(event)
    local map = function(keys, func, desc, mode)
      mode = mode or 'n'
      vim.keymap.set(mode, keys, func, { buffer = event.buf, desc = 'LSP: ' .. desc })
    end

    map('gd', require('telescope.builtin').lsp_definitions, '[G]oto [D]efinition')
    map('gr', require('telescope.builtin').lsp_references, '[G]oto [R]eferences')
    map('gI', require('telescope.builtin').lsp_implementations, '[G]oto [I]mplementation')
    map('<leader>D', require('telescope.builtin').lsp_type_definitions, 'Type [D]efinition')
    map('<leader>ds', require('telescope.builtin').lsp_document_symbols, '[D]ocument [S]ymbols')
    map('<leader>ws', require('telescope.builtin').lsp_dynamic_workspace_symbols, '[W]orkspace [S]ymbols')
    map('<leader>rn', vim.lsp.buf.rename, '[R]e[n]ame')
    map('<leader>ca', vim.lsp.buf.code_action, '[C]ode [A]ction', { 'n', 'x' })

    -- Declaration, not definition: in C this is the header.
    map('gD', vim.lsp.buf.declaration, '[G]oto [D]eclaration')

    local client = vim.lsp.get_client_by_id(event.data.client_id)

    -- supports_method moved from a method to a field across neovim versions;
    -- called the old way on a new version it errors, so ask carefully.
    local function supports(method)
      if not client then
        return false
      end
      if type(client.supports_method) == 'function' then
        local ok, res = pcall(client.supports_method, client, method)
        return ok and res
      end
      return false
    end

    -- Highlight the other uses of whatever the cursor is resting on.
    if supports(vim.lsp.protocol.Methods.textDocument_documentHighlight) then
      local highlight_augroup = vim.api.nvim_create_augroup('kiwami-lsp-highlight', { clear = false })
      vim.api.nvim_create_autocmd({ 'CursorHold', 'CursorHoldI' }, {
        buffer = event.buf,
        group = highlight_augroup,
        callback = vim.lsp.buf.document_highlight,
      })

      vim.api.nvim_create_autocmd({ 'CursorMoved', 'CursorMovedI' }, {
        buffer = event.buf,
        group = highlight_augroup,
        callback = vim.lsp.buf.clear_references,
      })

      vim.api.nvim_create_autocmd('LspDetach', {
        group = vim.api.nvim_create_augroup('kiwami-lsp-detach', { clear = true }),
        callback = function(event2)
          vim.lsp.buf.clear_references()
          vim.api.nvim_clear_autocmds { group = 'kiwami-lsp-highlight', buffer = event2.buf }
        end,
      })
    end

    if supports(vim.lsp.protocol.Methods.textDocument_inlayHint) then
      map('<leader>th', function()
        vim.lsp.inlay_hint.enable(not vim.lsp.inlay_hint.is_enabled { bufnr = event.buf })
      end, '[T]oggle Inlay [H]ints')
    end
  end,
})

-- Start them.
--
-- Two APIs exist depending on the version: neovim 0.11 moved server
-- definitions into `vim.lsp.config`/`vim.lsp.enable`, and nvim-lspconfig's
-- `require('lspconfig').<server>.setup` is the older path. Which one is
-- present depends on what nixpkgs is shipping this week, so use the new one
-- when it is there and fall back when it is not.
local new_api = vim.lsp.config ~= nil and vim.lsp.enable ~= nil

for server, cfg in pairs(servers) do
  cfg.capabilities = vim.tbl_deep_extend('force', {}, capabilities, cfg.capabilities or {})
  if new_api then
    vim.lsp.config(server, cfg)
    vim.lsp.enable(server)
  else
    require('lspconfig')[server].setup(cfg)
  end
end
