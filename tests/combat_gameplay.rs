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
                return {is_night=function() return night == true end, raining=clock_off}
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
        assert(strikes == nil and number == nil and state.phase == "recover")
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
        assert(strikes == 1 and number == nil)
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
fn an_exhausted_attack_does_not_advance_the_combo_or_schedule_damage() -> Result<()> {
    let lua = arena()?;
    lua.load(include_str!("../scripts/game/combat.lua")).exec()?;
    lua.load(r#"
        local charges = 0
        viber.vael.spend_attack_stamina = function() charges += 1 return false end
        viber.action = function(name) return name == "attack" end
        viber.play_clip = function() error("exhausted swing played") end
        viber.toast = noop
        on_update(0.016)
        assert(charges == 1 and state.swing == nil and state.attack_t == 0)
    "#).exec()
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

fn focus_arena() -> Result<Lua> {
    let lua = arena()?;
    lua.load(include_str!("../scripts/game/combat.lua"))
        .exec()?;
    lua.load(
        r#"
        viber.vael.hero_at_rest = function() return resting == true end
        viber.vael.combat_target = function(x,z) aimed_x, aimed_z = x,z end
        viber.action = function(name) return action == name end
        viber.ui.set = function(key, value)
            if key == "target.alive" then card = value end
            if key == "target.cue" then cue = value end
        end
        function enemy(x, z, chasing)
            return {x=x, y=0, z=z, hp=100, max=100, t=1, chasing=chasing}
        end
        function tick(dt)
            aimed_x, aimed_z = nil, nil
            on_update(dt or 0.1)
        end
    "#,
    )
    .exec()?;
    Ok(lua)
}

#[test]
fn passing_or_pausing_briefly_never_claims_the_camera() -> Result<()> {
    focus_arena()?
        .load(
            r#"
        game.enemies = {[1]=enemy(0,-1,true), [2]=enemy(0,6,true)}
        game.enemies[2].cue = "sidestep"
        for i=1, 20 do px = i/20; tick(); assert(not card and not aimed_z) end
        resting = true
        for i=1, 7 do tick(); assert(not aimed_z) end
        resting = false
        tick()
        resting = true
        for i=1, 7 do tick(); assert(not aimed_z) end
        for i=1, 3 do tick() end
        assert(aimed_z == 6 and cue == "sidestep" and state.locked_target == 2)
        resting = false
        tick()
        assert(not card and not aimed_z and not state.locked_target)
    "#,
        )
        .exec()
}

#[test]
fn idle_focus_requires_the_same_close_frontal_threat_during_active_play() -> Result<()> {
    focus_arena()?
        .load(
            r#"
        resting = true
        game.enemies = {[1]=enemy(0,-1,true), [2]=enemy(0,5,false), [3]=enemy(0,12,true)}
        for i=1, 14 do tick() end
        assert(not aimed_z)
        game.enemies[2].chasing = true
        viber.camera_state = function() return {yaw = yaw} end
        yaw = 90
        for i=1, 12 do tick(); assert(not aimed_z) end
        yaw = 0
        for i=1, 6 do tick() end
        game.enemies[2].hp = 0
        game.enemies[4] = enemy(0,6,true)
        for i=1, 6 do tick(); assert(not aimed_z) end
        viber.menu_open = function() return paused end
        paused = true; tick(3); paused = false
        for i=1, 6 do tick(); assert(not aimed_z) end
        for i=1, 4 do tick() end
        assert(state.locked_target == 4)
    "#,
        )
        .exec()
}

#[test]
fn accepted_attack_locks_its_victim_while_moving_and_manual_cycle_stays_available() -> Result<()> {
    focus_arena()?
        .load(
            r#"
        resting = false
        game.enemies = {[1]=enemy(0,0,true), [2]=enemy(0,3.8,false)}
        viber.vael.spend_attack_stamina = function() return stamina end
        viber.toast = noop
        action = "attack"; stamina = false; tick()
        assert(not aimed_z and not state.swing)
        stamina = true; tick()
        assert(state.focus_reason == "attack" and state.locked_target == 2 and aimed_z == 3.8)
        action = nil; tick(0.01)
        assert(state.locked_target == 2)
        game.enemies[2].chasing = true
        game.enemies[2].z = 18
        tick(0.11)
        assert(aimed_z == 18)
        action = "lock"; tick()
        assert(state.focus_reason == "manual" and state.locked_target == 1)
        action = nil
        game.enemies[1].invuln = true
        tick()
        assert(not aimed_z)
        game.enemies[2].z = 4
        tick()
        assert(not aimed_z)
        action = "lock"; tick()
        assert(aimed_z == 4)
        game.enemies[2].y = 4
        tick()
        assert(not aimed_z and not card)
    "#,
        )
        .exec()
}

#[test]
fn a_buffered_attack_only_locks_when_it_starts_and_radial_impact_can_claim_a_survivor() -> Result<()>
{
    let lua = focus_arena()?;
    lua.load("melee_update = on_update").exec()?;
    lua.load(include_str!("../scripts/game/abilities.lua"))
        .exec()?;
    lua.load("abilities_update = on_update; on_update = melee_update")
        .exec()?;
    lua.load(
        r#"
        game.enemies = {[1]=enemy(0,4,true), [2]=enemy(2,4,true)}
        action = "attack"; game.hero_dodge = {until_t=1.2}
        tick()
        assert(not aimed_z and state.buffer_queued)
        action = nil; game.hero_dodge = nil; tick(0.01)
        assert(aimed_x == 0 and aimed_z == 4 and state.swing)
        game.enemies[1].hp = 0
        tick(0.11)
        assert(not card)
        viber.burst, viber.kick, viber.punch = noop, noop, noop
        viber.input.pressed = function(key) return key == "r" end
        abilities_update(0.01)
        assert(game.strike_focus.id == 2 and game.enemies[2].hp < 100)
        tick(0.01)
        assert(aimed_x == 2 and state.focus_reason == "attack" and game.strike_focus == nil)
    "#,
    )
    .exec()
}

fn load_creature(source: &str) -> Result<Lua> {
    let lua = arena()?;
    lua.load(
        r#"
        viber.load = function(path)
            if path == "lib/rules.lua" then return rules end
            assert(path == "lib/fsm.lua")
            return fsm
        end
        viber.log = noop
        function third_swing()
            on_update(0.02)
            state.swings, state.t = 2, 5
            on_update(0.01)
            assert(state.kind == "special")
        end
        math.random = function() return 0.5 end
        viber.move_towards = noop
        viber.burst = noop
        viber.set_locomotion = noop
    "#,
    )
    .exec()?;
    lua.load(source).exec()?;
    Ok(lua)
}

#[test]
fn bandit_telegraphs_each_cut_and_a_parry_cancels_the_second() -> Result<()> {
    for parry in [false, true] {
        let lua = load_creature(include_str!("../scripts/enemies/bandit.lua"))?;
        lua.globals().set("parry", parry)?;
        lua.load(
            r#"
            on_update(0.02)
            state.swings, state.t = 2, 2
            on_update(0.01)
            assert(state.kind == "double" and state.pt >= 0.7)
            result = parry and {parried=true} or {entered=false}
            on_update(0.76)
            assert(strikes == 1)
            if parry then
                assert(state.phase == "stagger")
            else
                assert(state.kind == "followup" and state.pt >= 0.7)
                assert(math.abs(warning.z - pz) <= warning.r)
                pz = -pz
                on_update(0.73)
                assert(strikes == 1 and number == nil)
                assert(state.phase == "recover" and state.pt >= 1)
            end
        "#,
        )
        .exec()?;
    }
    Ok(())
}

#[test]
fn a_dodge_pressed_just_before_landing_fires_once_and_expires() -> Result<()> {
    let lua = arena()?;
    lua.load(include_str!("../scripts/game/abilities.lua"))
        .exec()?;
    lua.load(
        r#"
        local pressed, grounded, count = true, false, 0
        viber.input.pressed = function(key) return pressed and key == "c" end
        viber.vael.dodge = function() if grounded then return "side", 0.4 end end
        viber.vael.grant_iframes = function() count += 1 end
        viber.burst = noop
        on_update(0.016)
        assert(count == 0 and state.cd.dash == 0)
        pressed, grounded = false, true
        on_update(0.08)
        on_update(0.016)
        assert(count == 1 and state.cd.dash > 1)
        on_update(2)
        pressed, grounded = true, false
        on_update(0.016)
        pressed, grounded = false, true
        on_update(0.25)
        assert(count == 1 and state.cd.dash == 0)
    "#,
    )
    .exec()
}

fn charge_arena(source: &str) -> Result<Lua> {
    let lua = load_creature(source)?;
    lua.load(
        r#"
        viber.set_scale, viber.push_player = noop, noop
        viber.entity_knockback = function(id, dx, dz, strength)
            assert(dx == 0 and dz == 1)
            assert(hit_id == nil, "a charge can only knock back one enemy")
            hit_id, push = id, strength
        end
        local runner_z = 0
        viber.position = function() return 0, 0, runner_z end
        pz = 7
        on_update(0.01)
        state.ct = 10
        on_update(0.01)
        assert(state.kind == "charge" and state.phase == "windup")
        function mob(id, x, y, z, hp)
            game.enemies[id] = {x=x, y=y, z=z, hp=hp or 40, t=1, session=game.enemies[7].session}
            return game.enemies[id]
        end
        function launch()
            on_update(0.05)
            on_update(0.05)
            assert(game.enemies[7].cue == "bait_charge")
            on_update(0.85)
            assert(state.phase == "charging")
        end
        function travel(z, dt)
            runner_z = z
            on_update(dt or 0.1)
        end
    "#,
    )
    .exec()?;
    Ok(lua)
}

#[test]
fn sidestepped_charges_collide_once_in_travel_order() -> Result<()> {
    for (source, push, damage, recovery) in [
        (include_str!("../scripts/enemies/wolf.lua"), 4, 9, 1.0),
        (
            include_str!("../scripts/enemies/goblin-brute.lua"),
            7,
            16,
            1.4,
        ),
    ] {
        let lua = charge_arena(source)?;
        lua.globals().set("expected_push", push)?;
        lua.globals().set("damage", damage)?;
        lua.globals().set("recovery", recovery)?;
        lua.load(
            r#"
            mob(15, 0, 0, 0) -- packmate standing abreast
            mob(16, 0, 4, 2)
            mob(17, 0, 0, 3).boss = "boss"
            mob(18, 0, 0, 4).invuln = true
            mob(19, 2, 0, 5)
            mob(20, 0, 0, 5, 0)
            mob(21, 0, 0, 8.8)
            mob(22, 4, 0, 8)
            mob(23, 0, 0, 4)
            mob(24, 0, 0, 4)
            px = 3
            launch()
            game.enemies[22].x = 0 -- steps into the lane after launch
            game.enemies[23].t = -1
            game.enemies[24] = nil
            travel(2)
            assert(not hit_id)
            travel(9, 0.7)
            assert(hit_id == 22 and push == expected_push and strikes == nil and number == nil)
            assert(game.enemies[22].hp == 40 - damage and game.enemies[21].hp == 40)
            assert(state.phase == "recover" and state.pt == recovery)
            travel(9)
            assert(game.enemies[7].cue == "opening" and game.enemies[22].hp == 40 - damage)
        "#,
        )
        .exec()?;
    }
    Ok(())
}

#[test]
fn a_large_body_triggers_the_charge_at_its_physical_edge() -> Result<()> {
    let lua = charge_arena(include_str!("../scripts/enemies/wolf.lua"))?;
    lua.load(
        r#"
        mob(16, 0, 0, 8)
        viber.vael.contact_distance = function(a, b)
            assert(a == 7)
            if b == 16 then return 1.6 end
        end
        launch()
        px = 3
        travel(3)
        assert(hit_id == nil)
        travel(6.4)
        assert(hit_id == 16 and game.enemies[16].hp == 31 and strikes == nil)
        "#,
    )
    .exec()
}

#[test]
fn charge_hint_and_collision_follow_a_moving_lineup() -> Result<()> {
    let lua = charge_arena(include_str!("../scripts/enemies/wolf.lua"))?;
    lua.load(
        r#"
        local bystander = mob(14, 3, 0, 8)
        on_update(0.05)
        on_update(0.05)
        assert(game.enemies[7].cue == "sidestep")
        bystander.x = 0
        on_update(0.46)
        on_update(0.02)
        assert(game.enemies[7].cue == "bait_charge")
        on_update(0.3)
        assert(state.phase == "charging")
        travel(3)
        px = 3
        travel(6)
        assert(strikes == nil and hit_id == nil)
        bystander.x = 3
        travel(9)
        assert(strikes == nil and hit_id == nil and bystander.hp == 40)
    "#,
    )
    .exec()
}

#[test]
fn charge_respects_cover_and_a_dodge_through_the_lane() -> Result<()> {
    for (victim_z, dodged, collision) in [(4, false, true), (9, false, false), (9, true, true)] {
        let lua = charge_arena(include_str!("../scripts/enemies/wolf.lua"))?;
        lua.globals().set("victim_z", victim_z)?;
        lua.globals().set("dodged", dodged)?;
        lua.globals().set("collision", collision)?;
        lua.load(
            r#"
            mob(16, 0, 0, victim_z, 3)
            launch()
            result = {entered=not dodged, amount=10}
            travel(10, 1)
            assert((hit_id == 16) == collision)
            assert(game.enemies[16].hp == (collision and 1 or 3))
            assert((strikes or 0) == (victim_z < 7 and 0 or 1))
        "#,
        )
        .exec()?;
    }
    Ok(())
}

#[test]
fn wolf_charge_commits_to_its_lane_and_goblin_slam_leaves_an_opening() -> Result<()> {
    let lua = load_creature(include_str!("../scripts/enemies/wolf.lua"))?;
    lua.load(
        r#"
        pz = 7
        on_update(0.31)
        assert(state.phase == "windup" and state.kind == "charge" and strikes == nil)
        px, pz = 3, 1
        on_update(0.86)
        assert(state.phase == "charging" and math.abs(state.ax) < 0.01)
        viber.position = function() return 0, 0, 1 end
        on_update(0.1)
        assert(strikes == nil)
        on_update(2)
        assert(state.phase == "recover" and state.pt >= 1)
    "#,
    )
    .exec()?;
    let lua = load_creature(include_str!("../scripts/enemies/goblin.lua"))?;
    lua.load(
        r#"
        on_update(0.02)
        state.swings, state.t = 2, 2
        on_update(0.01)
        on_update(0.1)
        assert(state.kind == "special" and warning.x == 0 and warning.z == 0)
        pz = warning.r + 0.1
        on_update(0.91)
        assert(number == nil and strikes == nil)
        assert(state.phase == "recover" and state.pt >= 1)
    "#,
    )
    .exec()
}

#[test]
fn specializations_apply_the_documented_tradeoffs_in_combat() -> Result<()> {
    use vael::content::VaelContent;
    use vael::gear::{GearRules, GearState};
    use vael::stats::StatEngine;

    let content = VaelContent::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR")));
    let gear = GearRules::from_content(&content);
    let mut engine = StatEngine::new(Default::default());
    for (branch, mastery, key) in [
        ("strength", "embers", "r"),
        ("strength", "tremor", "r"),
        ("agility", "windstep", "c"),
        ("agility", "ghost", "c"),
    ] {
        let skills = [format!("{branch}1"), format!("{branch}2"), mastery.into()].into();
        let stats = engine.snapshot(&gear, &mut GearState::default(), &skills);
        let lua = arena()?;
        lua.globals().set("stats", stats.stats.clone())?;
        lua.globals().set("key", key)?;
        lua.globals().set("mastery", mastery)?;
        lua.load(include_str!("../scripts/game/abilities.lua"))
            .exec()?;
        lua.load(
            r#"
            viber.input.pressed = function(k) return k == key end
            viber.vael.stat = function(k) return stats[k] or 0 end
            viber.vael.player_stats = function() return {bonus_damage = stats.damage} end
            viber.vael.dodge = function() return "side", 0.3 end
            viber.vael.grant_iframes = function(seconds) window = seconds end
            viber.kick, viber.punch, viber.burst = noop, noop, noop
            game.enemies = {
                {hp=100, max=100, x=px, y=py, z=pz, t=1},
                {hp=100, max=100, x=px+5.5, y=py, z=pz, t=1}
            }
            on_update(0.016)
            local near, far = game.enemies[1], game.enemies[2]
            if mastery == "embers" then
                assert(near.hp == 25 and near.burn == 4 and far.hp == 100)
            elseif mastery == "tremor" then
                assert(near.hp == 13 and near.burn == nil and far.hp < 100)
            elseif mastery == "ghost" then
                assert(math.abs(state.cd.dash - 1.14) < 0.001 and math.abs(window - 0.55) < 0.001)
            else
                assert(math.abs(state.cd.dash - 0.84) < 0.001 and math.abs(window - 0.35) < 0.001)
            end
        "#,
        )
        .set_name(mastery)
        .exec()?;
    }
    Ok(())
}

#[test]
fn slime_marks_the_old_position_and_bogling_leaves_a_safe_center() -> Result<()> {
    let slime = load_creature(include_str!("../scripts/enemies/slime.lua"))?;
    slime
        .load(
            r#"
        third_swing()
        assert(warning.z == pz)
        px = warning.r + 0.2
        on_update(state.pt)
        assert(strikes == nil and state.phase == "recover")
    "#,
        )
        .exec()?;
    for (distance, hits) in [(1.0, 0), (2.4, 1), (4.0, 0)] {
        let bogling = load_creature(include_str!("../scripts/enemies/bogling.lua"))?;
        bogling.globals().set("distance", distance)?;
        bogling.globals().set("hits", hits)?;
        bogling
            .load(
                r#"
            third_swing()
            assert(warning.r == 1.65)
            pz = distance
            on_update(state.pt)
            assert((strikes or 0) == hits and state.phase == "recover")
        "#,
            )
            .exec()?;
    }
    Ok(())
}

#[test]
fn scorpion_lane_is_fixed_and_overlap_only_delivers_venom_once() -> Result<()> {
    for (side, height, immune, hits) in [
        (0., 0., false, 1),
        (1.3, 0., false, 0),
        (0., 3., false, 0),
        (0., 0., true, 1),
    ] {
        let lua = load_creature(include_str!("../scripts/enemies/scorpion.lua"))?;
        lua.globals().set("side", side)?;
        lua.globals().set("height", height)?;
        lua.globals().set("immune", immune)?;
        lua.globals().set("hits", hits)?;
        lua.load(
            r#"
            third_swing()
            assert(warning.x == 0 and warning.z == 3)
            px, py, pz = side, height, 1.5
            if immune then result = {entered=false} end
            on_update(state.pt)
            assert((strikes or 0) == hits)
            assert((game.venom or 0) == ((hits == 1 and not immune) and 4 or 0))
        "#,
        )
        .exec()?;
    }
    Ok(())
}

#[test]
fn striking_the_shield_invites_an_interruptible_counter() -> Result<()> {
    let lua = load_creature(include_str!("../scripts/enemies/bandit-shield.lua"))?;
    lua.load(
        r#"
        on_update(0.03)
        rules.damage_enemy(7, 2, true)
        on_update(0.1)
        assert(state.phase == nil and game.enemies[7].guard)
        state.t = 2
        on_update(0.02)
        on_update(0.02)
        assert(state.kind == "special" and not game.enemies[7].guard)
        assert(rules.hostiles(8)[1].cue == "shield_counter")
        rules.damage_enemy(7, 20)
        on_update(0.05)
        assert(state.phase == "stagger" and strikes == nil)
        on_update(0.01)
        assert(rules.hostiles(8)[1].cue == "opening")
    "#,
    )
    .exec()
}

#[test]
fn shades_echo_at_the_original_mark_with_a_safe_return_at_night() -> Result<()> {
    for (night, source) in [
        (false, include_str!("../scripts/enemies/shade.lua")),
        (true, include_str!("../scripts/enemies/shade-night.lua")),
    ] {
        let lua = load_creature(source)?;
        lua.globals().set("night", night)?;
        lua.load(
            r#"
            third_swing()
            local old_z = pz
            px = 2.3
            on_update(state.pt)
            assert(not strikes and state.kind == "echo" and state.ax == 0 and state.az == old_z)
            px = 0
            on_update(state.pt)
            assert((strikes or 0) == (night and 0 or 1))
            assert(state.phase == "recover")
        "#,
        )
        .exec()?;
    }
    let lua = load_creature(include_str!("../scripts/enemies/shade.lua"))?;
    lua.load(
        r#"
        third_swing()
        result = {parried=true}
        on_update(state.pt)
        assert(state.phase == "stagger" and state.kind ~= "echo")
    "#,
    )
    .exec()
}

#[test]
fn closing_on_an_archer_between_arrows_denies_the_second_shot() -> Result<()> {
    for (distance, arrows) in [(2., 1), (9., 2), (17., 1)] {
        let lua = load_creature(include_str!("../scripts/enemies/bandit-archer.lua"))?;
        lua.globals().set("distance", distance)?;
        lua.globals().set("arrows", arrows)?;
        lua.load(
            r#"
            local fired = 0
            viber.fire_projectile = function(proto)
                assert(proto == "enemy-arrow")
                fired += 1
            end
            pz = 8
            on_update(0.05)
            state.volleys, state.rt = 2, 3
            on_update(0.01)
            assert(state.kind == "paired_shot" and fired == 0)
            on_update(state.pt)
            assert(state.kind == "second_arrow" and fired == 1)
            pz = distance
            on_update(state.pt)
            assert(state.phase == "recover" and fired == arrows)
        "#,
        )
        .exec()?;
    }
    Ok(())
}

#[test]
fn counterbeat_answers_from_the_dodge_origin_once_per_target() -> Result<()> {
    use std::sync::{Arc, RwLock};
    use vael_engine::luau::game::{GameStore, game_to_json_store, install};

    let lua = arena()?;
    let store = Arc::new(RwLock::new(GameStore::default()));
    let host = lua.create_table()?;
    install(&lua, &host, Arc::clone(&store))?;
    let game: Table = host.get::<mlua::Function>("game")?.call(())?;
    lua.globals().set("game", game)?;
    lua.load(include_str!("../scripts/game/abilities.lua"))
        .exec()?;
    lua.load(r#"
        local tick, key, allowed, stamina, aboard, scene, toasts = 0, "", true, true, false, "arena", 0
        viber.time = function() return tick end
        viber.scene_name = function() return scene end
        viber.ui.get = function(k) return k == "boat.grip" and aboard and "oars" or "" end
        viber.input.pressed = function(k) return k == key end
        viber.vael.dodge = function() if allowed then return "roll", 0.5 end end
        viber.vael.spend_attack_stamina = function() return stamina end
        viber.vael.grant_iframes, viber.fov_kick = noop, noop
        viber.burst, viber.kick, viber.punch = noop, noop, noop
        viber.toast = function(msg) if string.find(msg, "Contratempo") then toasts += 1 end end
        viber.vael.player_stats = function() return {bonus_damage=6} end
        local function act(at, button)
            local dt = at - tick
            tick, key = at, button
            for _, enemy in pairs(game.enemies) do enemy.t = at end
            on_update(dt)
        end
        local function foe(z, y)
            return {hp=120, max=120, x=0, z=z, y=y or 0, t=tick}
        end
        local function roll()
            state, tick, pz = {}, 0, 0
            game.counterbeat = nil
            game.enemies = {foe(-1.8), foe(4.2), foe(0.5), foe(-1.8, 4)}
            act(0, "c")
            pz = 4.2
        end
        game["skill.strength1"], game["skill.agility1"] = true, true
        roll()
        act(0.5, "")
        assert(warning.z == 0 and warning.r == 2.8)
        act(0.75, "r")
        local function close(a,b) return math.abs(a-b) < 0.001 end
        assert(close(game.enemies[1].hp, 120-30*(1-0.5*1.8/2.8)))
        assert(close(game.enemies[2].hp, 120-66))
        assert(close(game.enemies[3].hp, 120-66*(1-0.5*3.7/4.8)))
        assert(game.enemies[4].hp == 120 and state.cd.strike == 8)
        assert(toasts == 1 and game["discovery.counterbeat"] and not game.counterbeat)
        roll()
        act(1.25, "r")
        assert(game.enemies[1].hp < 120 and toasts == 1)

        for _, at in ipairs({0.49, 1.26}) do
            roll()
            act(at, "r")
            assert(game.enemies[1].hp == 120 and game.enemies[2].hp == 54)
        end
        for _, skill in ipairs({"skill.strength1", "skill.agility1"}) do
            game[skill] = nil
            roll()
            act(0.6, "r")
            assert(game.enemies[1].hp == 120)
            game[skill] = true
        end
        allowed = false
        roll()
        act(0.6, "r")
        assert(game.enemies[1].hp == 120)
        allowed = true
        roll()
        stamina = false
        act(0.6, "r")
        assert(game.enemies[1].hp == 120 and state.cd.strike == 0)
        stamina = true
        act(0.8, "r")
        assert(game.enemies[1].hp < 120)
        for _, interrupt in ipairs({"boat","scene","menu","death","load"}) do
            roll()
            if interrupt == "boat" then aboard = true
            elseif interrupt == "scene" then scene = "other"
            elseif interrupt == "menu" then viber.menu_open = function() return true end
            elseif interrupt == "death" then game.hp = 0
            else game.session_id = (game.session_id or 0) + 1 end
            act(0.55, "")
            aboard, scene, viber.menu_open, game.hp = false, "arena", clock_off, 100
            act(0.65, "r")
            assert(game.enemies[1].hp == 120, interrupt)
        end
        roll()
        assert(game.counterbeat)
    "#).set_name("contratempo").exec()?;
    let saved = game_to_json_store(&store);
    assert_eq!(saved["discovery.counterbeat"], true);
    assert!(!saved.contains_key("counterbeat"));
    let flat = store.read().unwrap().snapshot();
    store.write().unwrap().replace(flat);
    lua.load("assert(game.counterbeat == nil and game['discovery.counterbeat'])").exec()
}

#[test]
fn lantern_off_hides_at_night_and_lantern_on_attracts_and_investigates() -> Result<()> {
    let lua = load_creature(include_str!("../scripts/enemies/bandit.lua"))?;
    lua.load(
        r#"
        local now = 1.0
        local lit = false
        local crouching = false
        local ex, ez = 0, 0
        local last_move_x, last_move_z
        viber.time = function() return now end
        viber.position = function() return ex, 0, ez end
        viber.wander_target = function() return 0, -5 end
        viber.next_state = function(cur, dist, aggro, deaggro)
            if cur == "chase" then
                return dist > deaggro and "wander" or "chase"
            end
            return dist <= aggro and "chase" or "wander"
        end
        viber.move_towards = function(tx, tz, speed)
            last_move_x, last_move_z = tx, tz
            local dx, dz = tx - ex, tz - ez
            local len = math.sqrt(dx * dx + dz * dz)
            if len > 1e-4 then
                local step = math.min(len, speed * 0.1)
                ex = ex + dx / len * step
                ez = ez + dz / len * step
            end
        end
        viber.vael.lantern_lit = function() return lit end
        viber.vael.hero_crouching = function() return crouching end

        -- À noite com lanterna apagada, a 7 m (aggro base = 10 m) o bandido NÃO ataca.
        night, lit, crouching = true, false, false
        px, pz = 0, 7.0
        for _ = 1, 5 do
            now += 0.1
            on_update(0.1)
        end
        assert(state.state == "wander", "lanterna apagada não deve atrair a 7 m")
        assert(state.light_inv_x == nil, "sem lanterna não investiga luz")

        -- Agachado no escuro a 3.5 m continua escondido (10 * 0.42 * 0.72 = 3.02 m).
        crouching = true
        ex, ez, px, pz = 0, 0, 0, 3.5
        now += 0.1
        on_update(0.1)
        assert(state.state == "wander", "agachado no escuro a 3.5 m não deve ser visto")
        crouching = false

        -- Com lanterna acesa à noite, a 17 m (halo de investigação 14.5..20.3 m)
        -- o bandido nota o clarão e avança na direção do herói.
        lit = true
        ex, ez, px, pz = 0, 0, 0, 17.0
        now += 0.1
        on_update(0.1)
        assert(state.state == "wander")
        assert(state.light_inv_z == 17.0 and ez > 0, "deve investigar o brilho da lanterna")

        -- Apagar a lanterna e sair de lado faz o bandido parar de rastrear o herói.
        lit = false
        px, pz = 12.0, 17.0
        now += 0.1
        on_update(0.1)
        assert(state.light_inv_x == 0 and state.light_inv_z == 17.0, "não rastreia no escuro")

        -- Com lanterna acesa a 13 m (aggro noite acesa = 14.5 m), entra logo em chase.
        lit = true
        ex, ez, px, pz = 0, 0, 0, 13.0
        now += 0.1
        on_update(0.1)
        assert(state.state == "chase", "lanterna acesa a 13 m deve atrair à noite")

        -- Apagar a lanterna a 11.5 m permite quebrar a perseguição (deaggro = 15 * 0.65 = 9.75 m).
        lit = false
        ex, ez, px, pz = 0, 0, 0, 11.5
        now += 0.1
        on_update(0.1)
        assert(state.state == "wander", "apagar a lanterna deve ajudar a despistar")
    "#,
    )
    .set_name("lantern_stealth")
    .exec()
}

#[test]
fn group_enemies_coordinate_turns_one_at_a_time_and_encircle() -> Result<()> {
    let lua = arena()?;
    lua.load(
        r#"
        local now = 1.0
        px, py, pz = 0, 0, 0
        local cur_id = 1
        local actors = {
            [1] = { x = 0, z = 4.0, st = {}, attacks = 0, was_att = false },
            [2] = { x = 3.5, z = 2.0, st = {}, attacks = 0, was_att = false },
            [3] = { x = -3.5, z = 2.0, st = {}, attacks = 0, was_att = false },
        }
        viber.time = function() return now end
        viber.self_id = function() return cur_id end
        viber.state = function() return actors[cur_id].st end
        viber.position = function() local a = actors[cur_id]; return a.x, 0, a.z end
        viber.next_state = function() return "chase" end
        viber.move_towards = function(tx, tz, speed)
            local a = actors[cur_id]
            local dx, dz = tx - a.x, tz - a.z
            local len = math.sqrt(dx * dx + dz * dz)
            if len > 1e-4 then
                local step = math.min(len, speed * 0.05)
                a.x = a.x + dx / len * step
                a.z = a.z + dz / len * step
            end
        end
        viber.burst, viber.set_locomotion, viber.set_scale = noop, noop, noop

        local band = fsm.new({
            kind = "bandit", elite_chance = 0,
            wander = 1.8, chase = 4.2, aggro = 12, deaggro = 18,
            range = 2.0, damage = 10, cooldown = 1.0, windup = 0.45, recover = 0.35,
        })

        local max_simultaneous = 0
        local waiting_samples = 0
        local waiting_ring_ok = 0

        for step_i = 1, 280 do
            now = now + 0.05
            for id = 1, 3 do
                cur_id = id
                band.update(0.05)
                local a = actors[id]
                local attacking = (a.st.phase == "windup" or a.st.phase == "charging")
                if attacking and not a.was_att then
                    a.attacks = a.attacks + 1
                end
                a.was_att = attacking
            end
            local simultaneous = 0
            for id = 1, 3 do
                local a = actors[id]
                local att = (a.st.phase == "windup" or a.st.phase == "charging")
                if att then
                    simultaneous = simultaneous + 1
                elseif step_i > 40 and a.st.phase == nil and game.combat_dir and game.combat_dir.attacker_id ~= id then
                    waiting_samples = waiting_samples + 1
                    local d = math.sqrt(a.x * a.x + a.z * a.z)
                    if d >= 2.2 then waiting_ring_ok = waiting_ring_ok + 1 end
                end
            end
            if simultaneous > max_simultaneous then max_simultaneous = simultaneous end
        end

        assert(max_simultaneous == 1, "nunca mais de 1 atacante simultâneo; got " .. max_simultaneous)
        for id = 1, 3 do
            assert(actors[id].attacks >= 2,
                "todos os 3 bandidos devem revezar turnos; id=" .. id .. " teve " .. actors[id].attacks)
        end
        assert(waiting_samples > 50 and waiting_ring_ok / waiting_samples > 0.75,
            string.format("inimigos em espera devem manter o anel de cerco (%.0f%%)",
                100 * waiting_ring_ok / math.max(1, waiting_samples)))
    "#,
    )
    .set_name("group_turns")
    .exec()
}

#[test]
fn mixed_group_coordinates_and_recovers_on_death_or_stagger() -> Result<()> {
    let lua = arena()?;
    lua.load(
        r#"
        local now = 1.0
        px, py, pz = 0, 0, 0
        local cur_id = 1
        local actors = {
            [1] = { x = 0, z = 2.0, st = {}, attacks = 0, was_att = false },
            [2] = { x = 0, z = 8.0, st = {}, attacks = 0, was_att = false },
            [3] = { x = 6.0, z = 0, st = {}, attacks = 0, was_att = false },
        }
        viber.time = function() return now end
        viber.self_id = function() return cur_id end
        viber.state = function() return actors[cur_id].st end
        viber.position = function() local a = actors[cur_id]; return a.x, 0, a.z end
        viber.next_state = function() return "chase" end
        viber.fire_projectile = noop
        viber.move_towards = function(tx, tz, speed)
            local a = actors[cur_id]
            local dx, dz = tx - a.x, tz - a.z
            local len = math.sqrt(dx * dx + dz * dz)
            if len > 1e-4 then
                local step = math.min(len, speed * 0.05)
                a.x = a.x + dx / len * step
                a.z = a.z + dz / len * step
            end
        end
        viber.burst, viber.set_locomotion, viber.set_scale = noop, noop, noop
        viber.corpse, viber.after, viber.entity_despawn, viber.push_player = noop, noop, noop, noop
        rules.award_kill = noop

        local melee = fsm.new({
            kind = "bandit", elite_chance = 0,
            wander = 1.8, chase = 4.0, aggro = 12, deaggro = 18,
            range = 2.0, damage = 10, cooldown = 1.1, windup = 0.5, recover = 0.35,
        })
        local archer = fsm.new({
            kind = "bandit-archer", elite_chance = 0,
            wander = 1.6, chase = 3.4, aggro = 16, deaggro = 22,
            range = 1.6, damage = 7, cooldown = 1.5, windup = 0.45, recover = 0.4,
            ranged = { range = 15, min = 5, cooldown = 1.5, windup = 0.5, projectile = "enemy-arrow" },
        })
        local wolf = fsm.new({
            kind = "wolf", elite_chance = 0,
            wander = 2.2, chase = 4.6, aggro = 12, deaggro = 18,
            range = 1.6, damage = 9, cooldown = 1.1, windup = 0.35, recover = 0.3,
            charge = { min = 4.5, max = 9, opening_delay = 0.4, cooldown = 4.0, windup = 0.7, speed = 8,
                       width = 1.1, mult = 1.15, recover = 0.8 },
        })
        local brains = { [1] = melee, [2] = archer, [3] = wolf }

        local killed_first = false
        for step_i = 1, 260 do
            now = now + 0.05
            local simultaneous = 0
            for id = 1, 3 do
                if not actors[id].dead then
                    cur_id = id
                    brains[id].update(0.05)
                    local a = actors[id]
                    local att = (a.st.phase == "windup" or a.st.phase == "charging")
                    if att then
                        simultaneous = simultaneous + 1
                        if not a.was_att then a.attacks = a.attacks + 1 end
                    end
                    a.was_att = att
                    -- Abate o primeiro atacante a meio do windup para garantir que não trava o grupo.
                    if not killed_first and id == 1 and a.st.phase == "windup" then
                        killed_first = true
                        game.enemies[1].hp = 0
                        brains[1].update(0.01)
                        a.dead = true
                    end
                end
            end
            assert(simultaneous <= 1, "grupo misto nunca pode atacar em simultâneo")
        end
        assert(killed_first, "bandido 1 deve ter iniciado ataque antes de cair")
        assert(actors[2].attacks >= 2, "arqueiro deve receber turnos; got " .. actors[2].attacks)
        assert(actors[3].attacks >= 2, "lobo deve receber turnos; got " .. actors[3].attacks)
    "#,
    )
    .set_name("mixed_group")
    .exec()
}

#[test]
fn spotting_player_alerts_nearby_squadmates_only_when_seen() -> Result<()> {
    let lua = arena()?;
    lua.load(
        r#"
        local now = 1.0
        local lit = false
        px, py, pz = 0, 0, 7.0
        local cur_id = 1
        local actors = {
            [1] = { x = 0, z = 0, st = {} },
            [2] = { x = 0, z = -6.0, st = {} }, -- a 13 m do herói, mas a 6 m do aliado 1
        }
        viber.time = function() return now end
        viber.self_id = function() return cur_id end
        viber.state = function() return actors[cur_id].st end
        viber.position = function() local a = actors[cur_id]; return a.x, 0, a.z end
        viber.wander_target = function() return 0, -10 end
        viber.move_towards = noop
        viber.next_state = function(cur, dist, aggro, deaggro)
            if cur == "chase" then return dist > deaggro and "wander" or "chase" end
            return dist <= aggro and "chase" or "wander"
        end
        viber.vael.lantern_lit = function() return lit end
        viber.vael.hero_crouching = function() return false end

        local band = fsm.new({
            kind = "bandit", elite_chance = 0,
            wander = 1.8, chase = 3.8, aggro = 10, deaggro = 15,
            range = 2.0, damage = 12, cooldown = 1.6,
        })

        night, lit = true, false
        for _ = 1, 4 do
            now += 0.1
            for id = 1, 2 do cur_id = id; band.update(0.1) end
        end
        assert(actors[1].st.state == "wander" and actors[2].st.state == "wander",
            "sem lanterna nenhum dos dois acorda")

        -- Acende a lanterna: o 1 avista a 7 m e alerta o companheiro 2 a 6 m dele.
        lit = true
        now += 0.1
        for id = 1, 2 do cur_id = id; band.update(0.1) end
        assert(actors[1].st.state == "chase", "bandido 1 deve ver a lanterna")
        assert(actors[2].st.state == "chase", "bandido 2 deve ouvir o chamado do grupo")
    "#,
    )
    .set_name("squad_callout")
    .exec()
}
