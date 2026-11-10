-- Rex Annotate: review documents and your agent's replies in plannotator-tui, and send the
-- feedback back to the agent as its next message.
--
-- Written by `plannotator-tui rex install`; run that again after upgrading plannotator-tui.
-- Load it from ~/.config/rex/init.lua, then run `rex config reload`:
--
--   require("plannotator.annotate").setup{}
--
-- Options, all optional:
--   placement = "split" | "popup"   where reviews open; default: [rex] placement in
--                                   plannotator-tui's config, else a split
--   keys = { annotate_gate = "ctrl+cmd+a", annotate_last = false }
--                                   change, add or drop keys; keys = false binds none
--   bin = "/path/to/plannotator-tui"

local M = {}

-- The plannotator-tui that wrote this file. Rex runs actions with a bare PATH, so the path is
-- absolute.
M.bin = @BIN@

-- Keys bound unless setup says otherwise. Rex for Mac binds none of them by default.
M.keys = {
  annotate_last = "shift+cmd+o",
  annotate_newest = "alt+shift+cmd+o",
  annotate_open = "ctrl+cmd+o",
}

-- Each action reviews the focused pane: the agent in it, its folder, or its output.
M.actions = {
  { name = "annotate_last", title = "Annotate: Agent's Last Reply", args = { "last" },
    description = "Review the agent's recent replies and send feedback back." },
  { name = "annotate_newest", title = "Annotate: Agent's Newest Reply", args = { "last", "--newest" },
    description = "Review the agent's newest reply, without the picker." },
  { name = "annotate_open", title = "Annotate: Documents Here", args = { "open" },
    description = "Review the Markdown files in the pane's folder and send feedback to its agent." },
  { name = "annotate_gate", title = "Annotate: Documents Here, with Approve", args = { "open", "--gate" },
    description = "Review the pane's folder with an Approve button, and send the decision to its agent." },
  { name = "annotate_terminal", title = "Annotate: Pane Output", args = { "terminal" },
    description = "Review the pane's recent output and send feedback to its agent." },
  { name = "annotate_find", title = "Annotate: Newest Markdown File Named Here", args = { "find" },
    description = "Review the newest Markdown file the pane's output names." },
}

local function launch(bin, placement, review, ctx)
  if ctx.block_id == nil then error("Annotate: no pane to review", 0) end
  local argv = { bin, "rex" }
  for _, arg in ipairs(review) do argv[#argv + 1] = arg end
  argv[#argv + 1] = "--block"
  argv[#argv + 1] = ctx.block_id
  if ctx.session_id ~= nil then
    argv[#argv + 1] = "--session"
    argv[#argv + 1] = ctx.session_id
  end
  if placement ~= nil then
    argv[#argv + 1] = "--placement"
    argv[#argv + 1] = placement
  end
  -- plannotator-tui opens the review pane and exits; the review runs in that pane.
  local r, err = rex.exec(argv, { timeout = 20 })
  if r == nil then error("Annotate: cannot run " .. bin .. ": " .. tostring(err), 0) end
  if not r.ok then
    local message = (r.stderr or ""):gsub("^%s+", ""):gsub("%s+$", "")
    if message == "" then message = "Annotate: plannotator-tui exited with status " .. tostring(r.code) end
    error(message, 0)
  end
end

function M.setup(opts)
  opts = opts or {}
  local bin = opts.bin or M.bin
  -- plannotator-tui checks the placement when a review opens.
  local placement = opts.placement

  for _, action in ipairs(M.actions) do
    local review = action.args
    rex.action{
      name = action.name,
      title = action.title,
      description = action.description,
      category = "Annotate",
      keywords = "plannotator review annotate feedback agent",
      run = function(ctx) launch(bin, placement, review, ctx) end,
    }
  end

  if opts.keys == false then return end
  local keys = {}
  for name, key in pairs(M.keys) do keys[name] = key end
  for name, key in pairs(opts.keys or {}) do keys[name] = key end
  -- `rex keymap` reports a key bound to an action that doesn't exist.
  for name, key in pairs(keys) do
    if key then rex.bind(key, name) end
  end
end

return M
