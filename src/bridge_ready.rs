//! Espelho de prontidão para a bridge `vael-tools`.
//!
//! O véu de carregamento é do monólito (`vael_engine::world_ready::WorldReadiness`);
//! o runtime da bridge (`bevy_ext_bridge_runtime`) é genérico e não conhece o
//! monólito. Este sistema copia o estado para o [`BridgeReadiness`] do
//! runtime, e o `vael-tools/ping` publica `ready` + `loading` na forma do
//! `vael.ping` — sem o espelho, o pong do runtime novo não tinha prontidão
//! nenhuma e o `wait-ready` do CLI declarava "pronto" com o véu ainda em pé.
//! Quando o monólito sair (fase H do DELETE-MONOLITH), o dono do véu passa a
//! escrever o recurso directamente; o contrato fica no runtime.

use bevy::prelude::*;

use bevy_ext_bridge_runtime::BridgeReadiness;

/// Plugin do espelho: insere o recurso e copia `WorldReadiness` a cada frame
/// (é um struct pequeno; copiar é mais barato do que detetar mudança).
pub struct BridgeReadyPlugin;

impl Plugin for BridgeReadyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BridgeReadiness>()
            .add_systems(Update, mirror);
    }
}

fn mirror(
    readiness: Option<Res<vael_engine::world_ready::WorldReadiness>>,
    mut bridge: ResMut<BridgeReadiness>,
) {
    let Some(r) = readiness else {
        return;
    };
    bridge.ready = r.ready;
    bridge.timed_out = r.timed_out;
    bridge.status = r.status_line();
    bridge.progress = r.shown_progress;
    bridge.last_load_secs = r.last_load_secs;
}
