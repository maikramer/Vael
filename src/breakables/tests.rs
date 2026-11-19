use super::*;

#[test]
fn authored_props_in_town_and_interiors_get_one_body_per_model() {
    fn count(node: &EntitySpec, totals: &mut [usize; 3]) {
        if let Some((_, kind)) = node.props.iter().find(|(k, _)| k == PROP) {
            let index = match kind.as_str() {
                "barrel" => 0,
                "crate" => 1,
                "stool" => 2,
                _ => unreachable!(),
            };
            assert!(Kind::asset(node).is_some() && node.physics.is_empty());
            totals[index] += 1;
        }
        if node
            .children
            .iter()
            .any(|c| c.props.iter().any(|(k, _)| k == PROP))
        {
            assert!(
                node.physics.is_empty(),
                "left a solid wrapper {:?}",
                node.name
            );
        }
        for child in &node.children {
            count(child, totals);
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (scene, minimum) in [
        ("world.bsn", [30, 30, 0]),
        ("scenes/interiors/house_a.bsn", [1, 1, 1]),
    ] {
        let doc = viber::xml::include::load_world_in(&root.join(scene), root).unwrap();
        let mut scene_data = viber::recipes::parse_world(&doc.root_attrs, &doc.nodes).unwrap();
        let mut totals = [0; 3];
        for node in &mut scene_data.entities {
            prepare_node(node);
            count(node, &mut totals);
        }
        eprintln!("{scene} barrel/crate/stool {totals:?}");
        assert!(totals.into_iter().zip(minimum).all(|(n, min)| n >= min));
    }
}

#[test]
fn melee_contact_respects_facing_reach_and_floor() {
    let s = Swing {
        at: Vec3::ZERO,
        aim: Vec2::X,
        reach: 2.8,
        arc: 0.,
        finisher: false,
    };
    assert!(s.reaches(Vec3::new(2., 0.5, 0.)));
    for pos in [Vec3::NEG_X, Vec3::X * 3., Vec3::new(1., 2.1, 0.)] {
        assert!(!s.reaches(pos));
    }
}

#[test]
fn walkover_only_pays_once_and_leaves_full_stacks() -> mlua::Result<()> {
    let lua = mlua::Lua::new();
    lua.load(r#"
        st = {}; hp=80; stock=0; hero=4; kind="life"; gone=false; capped=false
        local rules = {
          vault_add=function(_,n) if capped then return 0 end stock+=n return n end,
          item_label=function(k) return k end, player_hp=function() return true,hp,100 end
        }
        viber = {
          load=function() return rules end, prop=function(k) if k=="loot" then return kind end return k=="floor" and "0" or "0.45" end,
          state=function() return st end, menu_open=function() return false end, ui={is_open=function() return false end},
          entity_position=function() return true,0,0.45,0 end, player_position=function() return true,hero,0,0 end,
          set_pose=function() end, set_scale=function(v) assert(v>=0.05) end, despawn_self=function() gone=true end,
          vael={heal_player=function(n) hp=math.min(100,hp+n) return true end},
          burst=function() end, sound=function() end, damage_number=function() end,
        }
    "#).exec()?;
    lua.load(include_str!("../../scripts/game/prop_pickup.lua"))
        .exec()?;
    lua.load(
        r#"
        on_update(0.7); assert(hp==80 and not st.taken)
        hero=0; on_update(0.02); assert(hp==100 and st.taken and not gone)
        on_update(0.1); on_update(0.3); assert(gone and stock==0)
        for _, k in {"life","wood","gold"} do
          kind=k; st={}; gone=false; capped=true
          local prev=stock
          on_update(0.2); assert(not st.taken)
          on_update(0.5); assert(not st.taken and not gone and stock==prev)
          capped=false; on_update(0.1); on_update(0.1); on_update(0.4)
          assert(stock==prev+1 and gone)
        end
    "#,
    )
    .exec()
}
