//! HUD do Vael em Rust: o comportamento de `scripts/ui/hud.lua` que não é
//! diálogo.
//!
//! O Lua ficou só com o `say.tick`. As linhas de "outras missões", o clique do
//! diário, o TTL da barra de chefe e o soco do combo passam por aqui: os
//! setters vão pelo [`UiWriter`] (fila aplicada em `UiSet::Script`) e o único
//! bind que este módulo escreve é o `boss.active`, apagado quando o sinal do
//! `lib/fsm.lua` pára.

use std::time::Duration;

use bevy::prelude::*;
use viber::ui::script::{ScriptBindValue, UiScriptBinds};
use viber::ui::{UiClicks, UiRegistry, UiSet, UiWriter};

use crate::content::VaelContent;
use crate::hud::{OTHER_ROWS, best_quest, other_quest_rows, others_more_text};
use crate::save::VaelSave;

/// Mesmo TTL do `lib/fsm.lua` (`M.BOSS_BAR_TTL`): sem sinal há 1.5 s = chefe
/// morto, adormecido ou deixado para trás.
pub const BOSS_BAR_TTL: f64 = 1.5;

/// Período do refresh das "outras missões" (`OTHERS_PERIOD` do Lua).
pub const OTHERS_PERIOD: f32 = 0.5;

/// Plugin do HUD dirigido por Rust.
pub struct VaelHudUiPlugin;

impl Plugin for VaelHudUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                // Binds de script: antes de o `UiSet::Bind` publicar a vista
                // que os class-binds do XML leem.
                boss_bar_ttl.before(UiSet::Bind),
                // Os cliques são recolhidos no `UiSet::Bind` e os comandos
                // aplicam-se no `UiSet::Script`: estes correm no meio, no
                // mesmo frame.
                (refresh_other_quests, journal_click)
                    .after(UiSet::Bind)
                    .before(UiSet::Script),
                // `combo.text` é escrito pelo `combat.lua` dentro do
                // `luau_update` (o `UiSet::Script` já corre depois dele).
                combo_shake
                    .after(UiSet::Bind)
                    .after(viber::luau::luau_update)
                    .before(UiSet::Script),
            ),
        );
    }
}

/// TTL da barra de chefe: sem sinal fresco do FSM, `boss.active=false`.
///
/// O FSM (Lua) é o dono do `true`; aqui só se apaga, e nunca se reacende.
fn boss_bar_ttl(
    save: Res<VaelSave>,
    // `Time` (virtual) e não `Time<Real>`: o carimbo é escrito pelo
    // `lib/fsm.lua` com `viber.time()`, que é o `elapsed` do tempo virtual.
    // O hit-stop (`Time<Virtual>::relative_speed`) e a pausa do editor fazem
    // o relógio real afastar-se do virtual; comparar os dois esconderia a
    // barra para sempre depois de alguns golpes.
    time: Res<Time>,
    binds: Option<ResMut<UiScriptBinds>>,
) {
    let Some(mut binds) = binds else {
        return;
    };
    let seen = save
        .legacy
        .get("boss_bar_t")
        .and_then(serde_json::Value::as_f64);
    if !boss_bar_stale(seen, time.elapsed_secs_f64()) {
        return;
    }
    // Só toca no resource quando o valor muda: um `ResMut` mexido em todos os
    // frames re-publicava a vista inteira dos scripts.
    let already_off = binds.get("boss.active").is_some_and(|value| !value.flag());
    if !already_off {
        binds
            .0
            .insert("boss.active".to_string(), ScriptBindValue::Flag(false));
    }
}

/// `viber.time() - boss_bar_t >= TTL`, com os inválidos a contar como stale.
///
/// Um carimbo não finito (save antigo/corrompido) ou no "futuro" do relógio
/// desta sessão vale como sinal nenhum — a mesma regra do `hud.lua`.
pub fn boss_bar_stale(seen: Option<f64>, now: f64) -> bool {
    if !now.is_finite() {
        return true;
    }
    let Some(seen) = seen.filter(|n| n.is_finite()) else {
        return true;
    };
    if seen > now {
        return true;
    }
    now - seen >= BOSS_BAR_TTL
}

/// `refresh_other_quests` do `hud.lua`: as três linhas por baixo do cartão.
fn refresh_other_quests(
    content: Option<Res<VaelContent>>,
    save: Res<VaelSave>,
    binds: Option<Res<UiScriptBinds>>,
    writer: Option<Res<UiWriter>>,
    registry: Option<Res<UiRegistry>>,
    time: Res<Time>,
    mut timer: Local<Option<Timer>>,
) {
    let (Some(content), Some(writer)) = (content.as_deref(), writer.as_deref()) else {
        return;
    };
    // As linhas `quest-other-*` eram do HUD declarativo; o HUD `bevy_pf`
    // (`HudVm::quest_others`) calcula-as sozinho. Sem esta guarda o writer
    // avisava "unknown element" e as linhas eram recalculadas a cada 0,5 s
    // para nada.
    if registry
        .as_deref()
        .is_none_or(|registry| registry.get("quest-other-1").is_none())
    {
        return;
    }
    if !others_due(&mut timer, &time) {
        return;
    }
    // Título do cartão: o que o `VaelHud` publicou em `quest.title`; sem
    // binds de script, o mesmo cálculo que ele usa para o escolher.
    let shown = binds
        .as_deref()
        .and_then(|binds| binds.get("quest.title"))
        .map(ScriptBindValue::text)
        .unwrap_or_else(|| best_quest(&save, content).0);
    let rows = other_quest_rows(&save, content, &shown);

    for i in 0..OTHER_ROWS {
        let id = format!("quest-other-{}", i + 1);
        match rows.get(i) {
            Some(row) => {
                writer.set_text(id.clone(), row.text());
                writer.set_visible(id.clone(), true);
                writer.toggle_class(id, "ready", row.ready);
            }
            None => {
                writer.set_text(id.clone(), "");
                writer.set_visible(id.clone(), false);
                writer.toggle_class(id, "ready", false);
            }
        }
    }
    let extra = rows.len().saturating_sub(OTHER_ROWS);
    writer.set_text("quest-other-more", others_more_text(extra));
    writer.set_visible("quest-other-more", extra > 0);
}

/// Refresh imediato no primeiro frame (o Lua começava com `others_t = 0`);
/// depois a cada [`OTHERS_PERIOD`].
fn others_due(timer: &mut Option<Timer>, time: &Time) -> bool {
    match timer {
        Some(timer) => {
            timer.tick(time.delta());
            timer.just_finished()
        }
        None => {
            let mut fresh = Timer::from_seconds(OTHERS_PERIOD, TimerMode::Repeating);
            fresh.tick(Duration::from_secs_f32(OTHERS_PERIOD));
            *timer = Some(fresh);
            true
        }
    }
}

/// O botão "DIÁRIO": abre o menu na aba das missões.
///
/// Abrir na última aba (a loja, por exemplo) fazia o clique parecer que não
/// tinha aberto as missões.
fn journal_click(clicks: Option<Res<UiClicks>>, writer: Option<Res<UiWriter>>) {
    let (Some(clicks), Some(writer)) = (clicks.as_deref(), writer.as_deref()) else {
        return;
    };
    if !clicks.0.iter().any(|id| id == "open-journal") {
        return;
    }
    writer.open("menu", true);
    writer.select_tab("menu", "quests");
}

/// Soco visual do combo: cada mudança de `combo.text` reinicia a animação.
fn combo_shake(
    binds: Option<Res<UiScriptBinds>>,
    writer: Option<Res<UiWriter>>,
    registry: Option<Res<UiRegistry>>,
    mut last: Local<Option<String>>,
) {
    let (Some(binds), Some(writer)) = (binds.as_deref(), writer.as_deref()) else {
        return;
    };
    // O `combo-text` do HUD declarativo já não existe (HUD bevy_pf); sem esta
    // guarda o writer enchia o log com "unknown element".
    if registry
        .as_deref()
        .is_none_or(|registry| registry.get("combo-text").is_none())
    {
        return;
    }
    let Some(value) = binds.get("combo.text") else {
        return;
    };
    let combo = value.text();
    if last.as_deref() == Some(combo.as_str()) {
        return;
    }
    *last = Some(combo.clone());
    writer.set_anim("combo-text", combo_anim(&combo));
}

/// `"shake 0.45 0.5"` com texto, `"none"` sem (limpa a animação).
pub fn combo_anim(combo: &str) -> &'static str {
    if combo.is_empty() {
        "none"
    } else {
        "shake 0.45 0.5"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boss_bar_ttl_fresco_velho_nao_finito_e_futuro() {
        // Fresco: dentro do TTL de 1.5 s.
        assert!(!boss_bar_stale(Some(99.0), 100.0));
        // No limite já vale como sinal nenhum (`< TTL` no Lua).
        assert!(boss_bar_stale(Some(98.5), 100.0));
        assert!(boss_bar_stale(Some(98.4), 100.0));
        // Sem carimbo / não finito: stale.
        assert!(boss_bar_stale(None, 100.0));
        assert!(boss_bar_stale(Some(f64::NAN), 100.0));
        assert!(boss_bar_stale(Some(f64::INFINITY), 100.0));
        // Carimbo de outra sessão (no "futuro"): stale.
        assert!(boss_bar_stale(Some(120.0), 100.0));
        // Relógio inválido: stale.
        assert!(boss_bar_stale(Some(99.0), f64::NAN));
    }

    #[test]
    fn combo_anim_liga_e_limpa() {
        assert_eq!(combo_anim(""), "none");
        assert_eq!(combo_anim("x3"), "shake 0.45 0.5");
    }

    #[test]
    fn plugin_corre_sem_ui_nem_lua() {
        // Sem `UiPlugin` (nem `UiWriter`/`UiScriptBinds`/`UiClicks`) e sem
        // `LuaScriptHost`: os sistemas não podem entrar em pânico.
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<VaelSave>()
            .add_plugins(VaelHudUiPlugin);
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(app.world().resource::<VaelSave>(), &VaelSave::default());
    }
}
