//! Invariantes do Vael para o harness `bevy_playtest`.
//!
//! O harness já tem oráculos genéricos (Transform finito, bounds globais,
//! mundo congelado, panics e `ERROR` no log). O que falta é o contrato do
//! jogo: vitais, progressão, inventário e diário. Em vez de depender de uma
//! API interna de violações, estes sistemas validam o estado e logam com o
//! target `vael_playtest`: o sink de logs do harness transforma `ERROR` numa
//! violação com o frame e o detalhe.
//!
//! Só correm durante um cenário (`BEVY_PLAYTEST_SCENARIO` definido) e depois
//! do mundo estar pronto — durante o carregamento os recursos ainda estão a
//! caminho. O throttle evita inundar o relatório com o mesmo problema.

use bevy::prelude::*;
use bevy::time::Real;

use crate::components::{Inventory, PlayerVitals, QuestLog, VaelPlayer, XpState};

/// Estados de quest que o `rules.lua`/`VaelQuestPlugin` produzem.
const QUEST_STATES: [&str; 4] = ["not_taken", "active", "ready", "done"];

/// Intervalo mínimo entre duas verificações (segundos de `Time<Real>`).
const CHECK_PERIOD_S: f32 = 5.0;

/// Limites do herói depois de o mundo estar pronto. O oráculo de bounds do
/// harness usa ±1000 em todo o mundo; aqui o alvo é só o herói e com folga
/// para o streaming (o mapa não chega a estes valores).
const PLAYER_MAX_XZ: f32 = 10_000.0;
const PLAYER_MIN_Y: f32 = -200.0;

pub struct VaelPlaytestPlugin;

impl Plugin for VaelPlaytestPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, check_invariants.run_if(world_is_ready));
    }
}

fn world_is_ready(readiness: Option<Res<viber::world_ready::WorldReadiness>>) -> bool {
    readiness.is_none_or(|readiness| readiness.ready)
}

fn check_invariants(
    time: Res<Time<Real>>,
    mut last: Local<Option<f32>>,
    players: Query<&GlobalTransform, With<VaelPlayer>>,
    vitals: Option<Res<PlayerVitals>>,
    xp: Option<Res<XpState>>,
    inventory: Option<Res<Inventory>>,
    quests: Option<Res<QuestLog>>,
) {
    let now = time.elapsed_secs();
    let due = last.is_none_or(|last| now - last >= CHECK_PERIOD_S);
    if !due {
        return;
    }
    *last = Some(now);

    // Mais de um herói é sempre bug. Zero é legítimo durante uma troca de
    // cena (a entidade antiga já saiu, a nova ainda não nasceu).
    let count = players.iter().count();
    if count > 1 {
        error!(
            target: "vael_playtest",
            invariant = "player_unique",
            count,
            "há mais do que um herói no mundo"
        );
    }
    for transform in &players {
        let position = transform.translation();
        if let Some(detail) = player_position_violation(position) {
            error!(
                target: "vael_playtest",
                invariant = "player_position",
                x = position.x,
                y = position.y,
                z = position.z,
                detail,
                "posição do herói inválida"
            );
        }
    }
    if let Some(vitals) = vitals.as_deref()
        && let Some(detail) = vitals_violation(vitals)
    {
        error!(
            target: "vael_playtest",
            invariant = "player_vitals",
            hp = vitals.hp,
            max_hp = vitals.max_hp,
            detail,
            "vitais do herói inválidos"
        );
    }
    if let Some(xp) = xp.as_deref()
        && let Some(detail) = xp_violation(xp)
    {
        error!(
            target: "vael_playtest",
            invariant = "progression",
            xp = xp.xp,
            xp_next = xp.xp_next,
            level = xp.level,
            skill_points = xp.skill_points,
            detail,
            "progressão inválida"
        );
    }
    if let Some(inventory) = inventory.as_deref()
        && let Some(detail) = inventory_violation(inventory)
    {
        error!(
            target: "vael_playtest",
            invariant = "inventory",
            detail,
            "inventário inválido"
        );
    }
    if let Some(quests) = quests.as_deref()
        && let Some(detail) = quests_violation(quests)
    {
        error!(
            target: "vael_playtest",
            invariant = "quests",
            detail,
            "diário de quests inválido"
        );
    }
}

/// `None` = posição aceitável; `Some(motivo)` = violação.
pub fn player_position_violation(position: Vec3) -> Option<String> {
    if !position.is_finite() {
        return Some("posição não finita".to_string());
    }
    if position.y < PLAYER_MIN_Y {
        return Some(format!("y={} abaixo do limite {PLAYER_MIN_Y}", position.y));
    }
    if position.x.abs() > PLAYER_MAX_XZ || position.z.abs() > PLAYER_MAX_XZ {
        return Some(format!(
            "x/z fora do limite {PLAYER_MAX_XZ}: ({}, {})",
            position.x, position.z
        ));
    }
    None
}

/// `None` = vitais aceitáveis; `Some(motivo)` = violação.
pub fn vitals_violation(vitals: &PlayerVitals) -> Option<String> {
    if !vitals.hp.is_finite() || !vitals.max_hp.is_finite() {
        return Some("hp/max_hp não finitos".to_string());
    }
    if vitals.max_hp <= 0.0 {
        return Some(format!("max_hp={} não positivo", vitals.max_hp));
    }
    if vitals.hp < 0.0 {
        return Some(format!("hp={} negativo", vitals.hp));
    }
    if vitals.hp > vitals.max_hp + 0.01 {
        return Some(format!(
            "hp={} acima de max_hp={}",
            vitals.hp, vitals.max_hp
        ));
    }
    None
}

/// `None` = progressão aceitável; `Some(motivo)` = violação.
pub fn xp_violation(xp: &XpState) -> Option<String> {
    if xp.xp < 0 {
        return Some(format!("xp={} negativo", xp.xp));
    }
    if xp.xp_next <= 0 {
        return Some(format!("xp_next={} não positivo", xp.xp_next));
    }
    if xp.level < 1 {
        return Some(format!("level={} abaixo de 1", xp.level));
    }
    if xp.skill_points < 0 {
        return Some(format!("skill_points={} negativo", xp.skill_points));
    }
    None
}

/// `None` = inventário aceitável; `Some(motivo)` = violação.
pub fn inventory_violation(inventory: &Inventory) -> Option<String> {
    for (id, count) in inventory.entries() {
        if count < 0 {
            return Some(format!("{id}={count} negativo"));
        }
    }
    None
}

/// `None` = diário aceitável; `Some(motivo)` = violação.
pub fn quests_violation(quests: &QuestLog) -> Option<String> {
    for (id, state) in &quests.states {
        if !QUEST_STATES.contains(&state.as_str()) {
            return Some(format!("quest {id} com estado desconhecido {state:?}"));
        }
    }
    if let Some(tracked) = &quests.tracked
        && !quests.states.contains_key(tracked)
    {
        return Some(format!("quest seguida {tracked:?} não existe no diário"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vitals(hp: f32, max_hp: f32) -> PlayerVitals {
        PlayerVitals { hp, max_hp }
    }

    #[test]
    fn posicao_do_heroi_aceita_o_intervalo_e_rejeita_o_resto() {
        assert!(player_position_violation(Vec3::new(0.0, 1.0, 0.0)).is_none());
        assert!(player_position_violation(Vec3::new(0.0, -500.0, 0.0)).is_some());
        assert!(player_position_violation(Vec3::new(20_000.0, 1.0, 0.0)).is_some());
        assert!(player_position_violation(Vec3::new(f32::NAN, 1.0, 0.0)).is_some());
    }

    #[test]
    fn vitais_rejeitam_nan_negativos_e_acima_do_maximo() {
        assert!(vitals_violation(&vitals(100.0, 100.0)).is_none());
        assert!(vitals_violation(&vitals(0.0, 100.0)).is_none());
        assert!(vitals_violation(&vitals(-1.0, 100.0)).is_some());
        assert!(vitals_violation(&vitals(101.0, 100.0)).is_some());
        assert!(vitals_violation(&vitals(f32::NAN, 100.0)).is_some());
        assert!(vitals_violation(&vitals(100.0, 0.0)).is_some());
    }

    #[test]
    fn progressao_rejeita_negativos_e_nivel_zero() {
        let ok = XpState {
            xp: 10,
            xp_next: 100,
            level: 2,
            skill_points: 1,
            skills: Default::default(),
        };
        assert!(xp_violation(&ok).is_none());
        let mut bad = ok.clone();
        bad.xp = -1;
        assert!(xp_violation(&bad).is_some());
        let mut bad = ok.clone();
        bad.level = 0;
        assert!(xp_violation(&bad).is_some());
        let mut bad = ok;
        bad.xp_next = 0;
        assert!(xp_violation(&bad).is_some());
    }

    #[test]
    fn inventario_rejeita_contagens_negativas() {
        let mut inventory = Inventory::default();
        assert!(inventory_violation(&inventory).is_none());
        inventory.gold = -1;
        assert!(inventory_violation(&inventory).is_some());
        let mut inventory = Inventory::default();
        inventory.items.insert("estranho".into(), -3);
        assert!(inventory_violation(&inventory).is_some());
    }

    #[test]
    fn diario_rejeita_estados_desconhecidos_e_tracked_orfao() {
        let mut quests = QuestLog::default();
        assert!(quests_violation(&quests).is_none());
        quests.states.insert("forja".into(), "active".into());
        quests.tracked = Some("forja".into());
        assert!(quests_violation(&quests).is_none());
        quests.states.insert("forja".into(), "explodida".into());
        assert!(quests_violation(&quests).is_some());
        quests.states.insert("forja".into(), "active".into());
        quests.tracked = Some("outra".into());
        assert!(quests_violation(&quests).is_some());
    }
}
