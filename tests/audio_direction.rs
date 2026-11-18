//! The battle score follows pursuit, including a boss leaving a mixed encounter.
use mlua::{Lua, Result};

#[test]
fn score_follows_an_engaged_living_enemy() -> Result<()> {
    let lua = Lua::new();
    lua.load(
        r#"
        game = {enemies = {[7] = {chasing = false}, [8] = {chasing = false, boss = "Bruxa"}}}
        state, cues = {}, {}
        candidates = {{id=7, hp=10}, {id=8, hp=10}}
        viber = {
            state = function() return state end, game = function() return game end,
            menu_open = function() return false end,
            cinematic_playing = function() return false end,
            scene_name = function() return "main" end,
            load = function() return {
                player_hp = function() return true, 100 end,
                hostiles = function() return candidates end,
            } end,
            music = {combat = function(active, boss)
                table.insert(cues, active and (boss and "boss" or "battle") or "off")
            end}
        }
    "#,
    )
    .exec()?;
    lua.load(include_str!("../scripts/game/audio.lua")).exec()?;
    lua.load(
        r#"
        on_update(0.3)
        assert(#cues == 0)
        game.enemies[7].chasing = true
        on_update(0.3)
        game.enemies[8].chasing = true
        on_update(0.3)
        game.enemies[8] = nil
        candidates = {{id=7, hp=10}}
        on_update(0.3)
        viber.cinematic_playing = function() return true end
        on_update(0.3)
        assert(table.concat(cues, ",") == "battle,boss,off,battle,off")
    "#,
    )
    .exec()
}
