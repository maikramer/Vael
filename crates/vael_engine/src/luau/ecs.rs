//! Escrita genérica por reflexão (`patch_component`) — SHIM (2026-11-15,
//! onda final do DELETE-MONOLITH). O maquinaria vive agora no
//! `bevy_ext_luau::ecs` (a tabela `viber.ecs` e o espelho por frame morreram
//! antes — sobrou o caminho de escrita que `set_scale`/`set_visible`/
//! `set_pose` usam); os caminhos históricos `viber::luau::ecs::*` continuam a
//! resolver. Os testes ficaram AQUI com o consumidor (usam os tipos do
//! monólito — vitais, projéteis, IA — através do plugin de reflexão local).

pub use bevy_ext_luau::ecs::*;


#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use crate::reflect::ReflectPlugin;
    use crate::vitals::Health;

    fn test_world() -> App {
        let mut app = App::new();
        app.add_plugins(ReflectPlugin);
        // O espelho só corre com scripts vivos.
        app.world_mut()
            .spawn(super::super::components::LuaScriptRef {
                path: "test.lua".into(),
            });
        app
    }

    #[test]
    fn paths_da_engine_batem_certo() {
        assert_eq!(
            <Transform as bevy::reflect::TypePath>::type_path(),
            TRANSFORM_PATH
        );
        assert_eq!(
            <Visibility as bevy::reflect::TypePath>::type_path(),
            VISIBILITY_PATH
        );
    }

    #[test]
    fn whitelist_toda_registada_e_componente() {
        let mut app = App::new();
        app.add_plugins(ReflectPlugin);
        let registry = app
            .world()
            .resource::<bevy::ecs::reflect::AppTypeRegistry>();
        let registry = registry.read();
        for path in WHITELIST {
            let registration = registry
                .get_with_type_path(path)
                .unwrap_or_else(|| panic!("{path} não registado"));
            assert!(
                registration.data::<ReflectComponent>().is_some(),
                "{path} sem ReflectComponent"
            );
        }
    }

    #[test]
    fn patch_altera_o_componente() {
        let mut app = test_world();
        let entity = app
            .world_mut()
            .spawn((
                Health {
                    current: 30.0,
                    max: 80.0,
                },
                Transform::default(),
            ))
            .id();

        let patch = EcsValue::Map(vec![("current".into(), EcsValue::Float(12.0))]);
        assert_eq!(
            patch_component(app.world_mut(), entity, "viber::vitals::Health", &patch),
            Ok(true)
        );
        let health = app.world().get::<Health>(entity).unwrap();
        assert!((health.current - 12.0).abs() < 1e-6);
        assert!((health.max - 80.0).abs() < 1e-6);

        // `set_pose`/`set_scale` internos: Transform não está na whitelist.
        let pose = EcsValue::Map(vec![
            ("translation".into(), EcsValue::Vec3([1.0, 2.0, 3.0])),
            (
                "rotation".into(),
                EcsValue::Quat([0.0, (0.5f32).sin(), 0.0, (0.5f32).cos()]),
            ),
        ]);
        assert_eq!(
            patch_component(app.world_mut(), entity, TRANSFORM_PATH, &pose),
            Ok(true)
        );
        let transform = app.world().get::<Transform>(entity).unwrap();
        assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
        assert!((transform.rotation.y - (0.5f32).sin()).abs() < 1e-6);
    }

    #[test]
    fn insert_e_remove_respeitam_a_construcao() {
        let mut app = test_world();
        let entity = app.world_mut().spawn(Transform::default()).id();
        let patch = EcsValue::Map(vec![
            ("current".into(), EcsValue::Float(5.0)),
            ("max".into(), EcsValue::Float(9.0)),
        ]);
        insert_component(app.world_mut(), entity, "viber::vitals::Health", &patch).unwrap();
        let health = app.world().get::<Health>(entity).unwrap();
        assert_eq!((health.current, health.max), (5.0, 9.0));

        // Unitário: `Hostile` sem campos (Default do próprio tipo).
        insert_component(
            app.world_mut(),
            entity,
            "viber::vitals::Hostile",
            &EcsValue::Unit,
        )
        .unwrap();
        assert!(app.world().get::<crate::vitals::Hostile>(entity).is_some());

        // Campo inexistente: erro claro, sem tocar no componente.
        let bad = EcsValue::Map(vec![("nope".into(), EcsValue::Float(1.0))]);
        assert!(
            patch_component(app.world_mut(), entity, "viber::vitals::Health", &bad)
                .unwrap_err()
                .contains("campo 'nope'")
        );

        assert_eq!(
            remove_component(app.world_mut(), entity, "viber::vitals::Health"),
            Ok(true)
        );
        assert!(app.world().get::<Health>(entity).is_none());
    }

    #[test]
    fn insert_sem_default_constroi_todos_os_campos() {
        let mut app = test_world();
        let entity = app.world_mut().spawn(Transform::default()).id();
        let patch = EcsValue::Map(vec![
            (
                "vel".into(),
                EcsValue::Map(vec![
                    ("x".into(), EcsValue::Float(1.0)),
                    ("y".into(), EcsValue::Float(2.0)),
                    ("z".into(), EcsValue::Float(3.0)),
                ]),
            ),
            ("life".into(), EcsValue::Float(2.0)),
            ("damage".into(), EcsValue::Float(5.0)),
            ("sensor_radius".into(), EcsValue::Float(0.1)),
            ("faction".into(), EcsValue::Str("Enemy".into())),
            ("gravity".into(), EcsValue::Float(9.8)),
            ("shooter".into(), EcsValue::Entity(entity.to_bits() as i64)),
        ]);
        insert_component(
            app.world_mut(),
            entity,
            "bevy_ext_viber_engine::projectile::Projectile",
            &patch,
        )
        .expect("insert sem Default");
        let projectile = app
            .world()
            .get::<crate::projectile::Projectile>(entity)
            .unwrap();
        assert_eq!(projectile.vel, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(projectile.faction, crate::projectile::Faction::Enemy);
        assert_eq!(projectile.shooter, Some(entity));

        // Sem um campo e sem Default: erro claro (não constrói a zeros).
        let partial = EcsValue::Map(vec![("vel".into(), EcsValue::Vec3([0.0, 0.0, 0.0]))]);
        let error = insert_component(
            app.world_mut(),
            entity,
            "bevy_ext_viber_engine::projectile::Projectile",
            &partial,
        )
        .unwrap_err();
        assert!(error.contains("faltam campos"), "{error}");
    }

    #[test]
    fn patch_option_e_enum_por_nome() {
        let mut app = test_world();
        let entity = app
            .world_mut()
            .spawn((crate::ai::EnemyCreature::default(), Transform::default()))
            .id();
        let patch = EcsValue::Map(vec![
            (
                "home".into(),
                EcsValue::Map(vec![
                    ("x".into(), EcsValue::Float(3.0)),
                    ("y".into(), EcsValue::Float(-4.0)),
                ]),
            ),
            ("state".into(), EcsValue::Str("Chase".into())),
            ("aggro_radius".into(), EcsValue::Float(11.0)),
        ]);
        assert_eq!(
            patch_component(app.world_mut(), entity, "viber::ai::EnemyCreature", &patch),
            Ok(true)
        );
        let creature = app.world().get::<crate::ai::EnemyCreature>(entity).unwrap();
        assert_eq!(creature.home, Some(Vec2::new(3.0, -4.0)));
        assert_eq!(creature.state, crate::ai::EnemyState::Chase);
        assert_eq!(creature.aggro_radius, 11.0);

        // `Option` de volta a `None` por "none".
        let patch = EcsValue::Map(vec![("home".into(), EcsValue::Str("none".into()))]);
        patch_component(app.world_mut(), entity, "viber::ai::EnemyCreature", &patch).unwrap();
        assert_eq!(
            app.world()
                .get::<crate::ai::EnemyCreature>(entity)
                .unwrap()
                .home,
            None
        );
    }
}
