//! Regressões nas regras Luau que ligam a mira, o escudo e a IA do jogo.
use mlua::{Lua, Result, Table};

fn arena() -> Result<Lua> {
    let lua = Lua::new();
    lua.load(
        r#"
        game, state = {hp=100, max_hp=100}, {}
        px, py, pz = 0, 0, 1.5
        function noop() end
        function clock_off() return false end
        viber = {
            game = function() return game end, state = function() return state end,
            time = function() return 1 end, self_id = function() return 7 end,
            position = function() return 0, 0, 0 end,
            player_position = function() return true, px, py, pz end,
            player_id = function() return 9 end,
            player_forward = function() return 0, 1 end,
            menu_open = clock_off, cinematic_playing = clock_off,
            ui = {set=noop, get=function() return "" end}, input = {pressed=clock_off},
            next_state = function() return "chase" end,
            ring = function(x,z,r) warning = {x=x,z=z,r=r} end,
            damage_number = function(label) number = label end,
            face_player=noop, face_towards=noop, play_clip=noop, sound=noop,
            shake=noop, hit_stop=noop, load_json=function() return {} end,
            load = function(path)
                if path == "lib/rules.lua" then return rules end
                if path == "lib/gear.lua" then
                    return {weapon=function() return {speed=1.4, range=3, damage=25} end}
                end
                return {is_night=clock_off, raining=clock_off}
            end,
            vael = {
                damage_player = function()
                    strikes = (strikes or 0) + 1
                    return result or {entered=true, amount=10}
                end,
                player_stats=function() return {} end, stat=function() return 0 end,
            },
        }
        "#,
    )
    .exec()?;
    for (name, source) in [
        ("rules", include_str!("../scripts/lib/rules.lua")),
        ("fsm", include_str!("../scripts/lib/fsm.lua")),
    ] {
        let module: Table = lua.load(source).set_name(name).eval()?;
        lua.globals().set(name, module)?;
    }
    lua.load(
        r#"
        foe = fsm.new({elite_chance=0, cooldown=0.1, windup=0.4, shield={reduce=0.8}})
        function windup()
            foe.update(0.11)
            assert(state.phase == "windup")
        end
        "#,
    )
    .exec()?;
    Ok(lua)
}

#[test]
fn aproxima_o_golpe_sem_travar_a_cadencia_num_colisor() -> Result<()> {
    arena()?
        .load(
            r#"
        pz = 1.85
        viber.move_towards = function(x, z, speed) goal = z end
        foe.update(0.05)
        assert(goal > 0 and goal < pz)
        -- A posição não muda (cápsula encostada), mas o golpe ainda arranca.
        foe.update(0.06)
        assert(state.phase == "windup")
    "#,
        )
        .exec()
}

#[test]
fn golpe_normal_respeita_o_aviso_mesmo_depois_de_mudar_de_lado() -> Result<()> {
    arena()?
        .load(
            r#"
        windup()
        assert(math.abs(pz-warning.z) < warning.r)
        pz = -pz
        assert(math.abs(pz-warning.z) > warning.r)
        foe.update(0.4)
        assert(strikes == nil and number == "ESQUIVA" and state.phase == "recover")
    "#,
        )
        .exec()
}

#[test]
fn aparo_abre_o_contra_ataque_so_no_atacante() -> Result<()> {
    arena()?
        .load(
            r#"
        windup()
        result = {parried=true}
        foe.update(0.4)
        assert(strikes == 1 and number == "APARO")
        assert(state.phase == "stagger" and state.pt >= 0.8)
        foe.update(0.01)
        assert(not game.enemies[7].guard)
    "#,
        )
        .exec()
}

#[test]
fn escudo_aguenta_o_raspao_mas_nao_esconde_um_segundo_impacto() -> Result<()> {
    arena()?
        .load(
            r#"
        windup()
        rules.damage_enemy(7, 5, true)
        foe.update(0.05)
        assert(state.phase == "windup" and game.enemies[7].hp == 95)
        rules.damage_enemy(7, 5, true)
        rules.damage_enemy(7, 20)
        foe.update(0.05)
        assert(state.phase == "stagger")
        local killed, _, removed = rules.damage_enemy(7, 999)
        assert(killed and removed == 70)
    "#,
        )
        .exec()
}

#[test]
fn impacto_e_queimadura_partilham_altura_e_invulnerabilidade() -> Result<()> {
    arena()?
        .load(
            r#"
        game.enemies = {}
        for id = 1, 3 do
            game.enemies[id] = {hp=100, max=100, x=0, z=0, y=0, t=1}
        end
        game.enemies[2].y = 3
        game.enemies[3].invuln = true
        assert(rules.radial_damage(0,0,5,20,0,0,3) == 1)
        assert(game.enemies[1].hp == 80 and game.enemies[1].burn == 3)
        for id = 2, 3 do
            assert(game.enemies[id].hp == 100 and game.enemies[id].burn == nil)
        end
    "#,
        )
        .exec()
}

#[test]
fn esquiva_consume_o_buffer_uma_vez_e_a_tocha_cancela_o_impacto() -> Result<()> {
    let lua = arena()?;
    lua.load(include_str!("../scripts/game/combat.lua"))
        .set_name("combat")
        .exec()?;
    lua.load(
        r#"
        local presses, torch, swings = false, false, 0
        viber.action = function(name) return presses and name == "attack" end
        viber.ui.get = function(key) return key == "equip.kind" and torch and "torch" or "" end
        viber.player_id = function() return 1 end
        viber.play_clip = function() swings += 1 end
        viber.toast = noop
        game.hero_dodge = {until_t=1.1}
        presses = true
        on_update(0.016)
        assert(swings == 0 and state.buffer_queued)
        game.hero_dodge = nil
        on_update(0.016)
        assert(swings == 1 and not state.buffer_queued)
        presses, torch = false, true
        on_update(0.2)
        assert(state.swing == nil and state.combo_hits == 0)
        torch = false
        on_update(0.35)
        assert(swings == 1)
    "#,
    )
    .exec()
}

#[test]
fn boarding_discards_buffered_melee_and_landing_accepts_a_fresh_attack() -> Result<()> {
    let lua = arena()?;
    lua.load(include_str!("../scripts/game/combat.lua"))
        .exec()?;
    lua.load(
        r#"
        local aboard, pressed, swings = false, true, 0
        viber.ui.get = function(key) return key == "boat.grip" and aboard and "oars" or "" end
        viber.input.pressed = function(key) return pressed and (key == "j" or key == "lmb") end
        viber.action = function(name) return pressed and name == "attack" end
        viber.play_clip = function() swings += 1 end
        game.hero_dodge = {until_t=1.1}
        on_update(0.016)
        assert(state.buffer_queued)
        game.hero_dodge, aboard = nil, true
        on_update(0.016)
        assert(swings == 0 and state.swing == nil and not state.buffer_queued)
        aboard, pressed = false, false
        on_update(0.3)
        assert(swings == 0)
        pressed = true
        on_update(0.016)
        assert(swings == 1)
    "#,
    )
    .exec()
}

#[test]
fn surviving_an_unblocked_hit_plays_the_hero_reaction_once() -> Result<()> {
    arena()?
        .load(
            r#"
        local clips = {}
        viber.play_clip = function(name, opts)
            assert(opts.id == viber.player_id())
            table.insert(clips, name)
        end
        result = {entered=true, amount=10}
        rules.damage_player(10)
        assert(#clips == 1 and clips[1] == "hit")
        result = {entered=true, blocked=true, amount=3}
        rules.damage_player(10)
        result = {entered=false, amount=0}
        rules.damage_player(10)
        assert(#clips == 1)
    "#,
        )
        .exec()
}
