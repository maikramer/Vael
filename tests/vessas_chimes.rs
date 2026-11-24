//! Exercise the footbridge with the same Luau script that the world loads.
use mlua::{Lua, Result};

#[test]
fn both_crossings_play_and_only_a_pause_on_the_deck_gets_an_answer() -> Result<()> {
    let lua = Lua::new();
    lua.load(
        r#"
        game, state, pitches, answered, stories = {hp=100}, {}, {}, 0, 0
        x, y, z = 0, 4, 0
        vael = {
            state=function() return state end, game=function() return game end,
            position=function() return 0, 0, 0 end, self_id=function() return 1 end,
            player_position=function() return true, x, y, z end,
            menu_open=function() return paused end, after=function() end, burst=function() end,
            toast=function() stories += 1 end,
            sound_loop=function(_, clip, opts)
                if clip == "world/vessa_bronze.ogg" then table.insert(pitches,opts.pitch)
                elseif clip == "world/vessa_quarta_voz.ogg" then answered += 1
                else error(clip) end
            end,
        }
        function step(offset, height, seconds)
            z, y = offset, height
            for _=1, math.round(seconds*20) do on_update(.05) end
        end
    "#,
    )
    .exec()?;
    lua.load(include_str!("../scripts/poi/carrilhao-de-vessa.lua"))
        .exec()?;
    lua.load(
        r#"
        step(0,-2,4) -- A swimmer below the deck hears no bells.
        assert(#pitches == 0 and answered == 0)
        for _,dir in {1,-1} do
            for i=-16,16,.25 do
                step(dir*i, 1+.18*(16-dir*i)/32+2.75*math.cos(i*math.pi/32), .05)
            end
            step(18,1,.05)
        end
        assert(#pitches == 6 and answered == 0)
        for i=1,3 do assert(pitches[i] == pitches[7-i]) end
        assert(pitches[1] < pitches[3] and pitches[3] < pitches[2])
        step(0,3.8,.1)
        paused = true
        step(0,3.8,5)
        assert(answered == 0)
        paused = false
        step(0,3.8,10)
        assert(answered == 1 and stories == 1 and #pitches == 7)
        step(18,1,.1)
        step(0,3.8,3)
        assert(answered == 2 and stories == 1)
    "#,
    )
    .exec()
}
