-- HUD: vida/XP/economia/quests/chefe/combo passaram para o Rust (VaelHud).
local say = vael.load("ui/say.lua")
function on_update(dt)
  say.tick(dt)
end
