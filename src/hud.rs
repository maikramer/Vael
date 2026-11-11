//! HUD do Vael calculado em Rust — shadow mode.
//!
//! O `scripts/lib/rules.lua` continua a publicar os mesmos binds. Este módulo
//! calcula-os outra vez a partir do estado tipado e compara-os com o que o Lua
//! deixou em [`UiScriptBinds`]: divergência = `debug!` por chave e `warn!` uma
//! vez. O último escritor vence — o Lua reescreve no fim do frame, por isso só
//! um bind divergente aparece por UM frame. `VAEL_HUD_RUST=0` desliga-o.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde_json::Value;
use viber::ui::UiSet;
use viber::ui::script::{ScriptBindValue, UiScriptBinds};

use crate::content::{Objective, QuestDef, VaelContent};
use crate::save::VaelSave;

/// Variável de ambiente do gate do HUD em Rust (`1` = ligado).
pub const HUD_RUST_ENV: &str = "VAEL_HUD_RUST";

/// Constante do `rules.lua` (`FLASK_BASE`): máximo base do cantil.
const FLASK_BASE: i64 = 2;

/// Gate do espelho do HUD, lido do ambiente no arranque.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaelHudConfig {
    pub rust: bool,
}

impl Default for VaelHudConfig {
    fn default() -> Self {
        Self {
            rust: env_flag(HUD_RUST_ENV),
        }
    }
}

fn env_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(raw) => !matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
        Err(_) => true,
    }
}

/// Plugin do HUD em Rust.
pub struct VaelHudPlugin;

impl Plugin for VaelHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VaelHudConfig>().add_systems(
            Update,
            publish_hud_binds
                // O espelho de estado corre no `PreUpdate` (frame anterior do
                // Lua); publicar antes do `UiSet::Bind` faz os `bind=` deste
                // frame lerem já os valores Rust, sem ciclo com `luau_update`.
                .before(UiSet::Bind),
        );
    }
}

/// Escreve os binds Rust no [`UiScriptBinds`] e compara-os com os do Lua.
fn publish_hud_binds(
    config: Res<VaelHudConfig>,
    save: Res<VaelSave>,
    content: Option<Res<VaelContent>>,
    // `Option`: apps sem UI (ou sem o `UiPlugin`) não têm o recurso.
    binds: Option<ResMut<UiScriptBinds>>,
    // Última publicação Rust: entre snapshots de 10 Hz não se reescrevem os
    // valores (mais frescos) que o Lua deixou no mapa.
    mut last: Local<Option<BTreeMap<String, ScriptBindValue>>>,
    mut warned: Local<bool>,
    mut logged: Local<bool>,
) {
    if !config.rust {
        if !*logged {
            *logged = true;
            info!("vael HUD: desligado por VAEL_HUD_RUST");
        }
        return;
    }
    let (Some(content), Some(mut binds)) = (content, binds) else {
        if !*logged {
            *logged = true;
            warn!("vael HUD: VaelContent/UiScriptBinds ausentes — sem publicação");
        }
        return;
    };
    let computed = publish_binds(&save, &content);
    if !*logged {
        *logged = true;
        info!(
            binds = computed.len(),
            first = last.is_none(),
            "vael HUD: primeira publicação"
        );
    }

    let mut changed = Vec::new();
    let mut divergem = 0usize;
    for (name, value) in &computed {
        // `is_some_and`: sem valor anterior (primeiro publish) o bind MUDOU.
        // `is_none_or` devolvia true e saltava todos no arranque.
        let igual_ao_rust = last
            .as_ref()
            .and_then(|prev| prev.get(name))
            .is_some_and(|prev| same_bind(prev, value));
        if igual_ao_rust {
            continue;
        }
        changed.push((name.clone(), value.clone()));
        if let Some(current) = binds.0.get(name)
            && !same_bind(current, value)
        {
            debug!("vael HUD: `{name}` diverge do Lua ({current:?} != {value:?})");
            divergem += 1;
        }
    }
    if changed.is_empty() {
        return;
    }
    if divergem > 0 && !*warned {
        *warned = true;
        warn!(
            binds = divergem,
            "vael HUD: o espelho Rust diverge do Lua (shadow mode) — ver `debug`"
        );
    }
    for (name, value) in changed {
        binds.0.insert(name, value);
    }
    *last = Some(computed.into_iter().collect());
}

/// Calcula os binds do HUD a partir do estado tipado.
///
/// Os nomes e a forma dos valores são os de `rules.publish`: `Number` para
/// barras/quantidades, `Text` para rótulos e `Flag` para classes/visibilidade.
pub fn publish_binds(save: &VaelSave, content: &VaelContent) -> Vec<(String, ScriptBindValue)> {
    let mut out = Vec::with_capacity(32);

    let hp = save.vitals.hp as f64;
    let max_hp = save.vitals.max_hp as f64;
    let health = if max_hp > 0.0 {
        (hp / max_hp).clamp(0.0, 1.0)
    } else {
        0.0
    };
    push(&mut out, "health", ScriptBindValue::Number(health));
    push(
        &mut out,
        "health.text",
        ScriptBindValue::Text(format!("{}/{}", arredonda(hp), arredonda(max_hp))),
    );
    push(&mut out, "health.value", ScriptBindValue::Number(hp));
    push(&mut out, "health.low", ScriptBindValue::Flag(health <= 0.3));

    for (name, key) in [
        ("status.venom", "venom"),
        ("status.burn", "burn"),
        ("status.frost", "frost"),
    ] {
        push(&mut out, name, ScriptBindValue::Flag(num(save, key) > 0.0));
    }

    // `M.flasks()`: floor e mínimo 0. Sem a chave, o Lua publica o máximo
    // base (2 + talentos); aqui só há o base — a divergência fica no log.
    let flasks = save
        .legacy
        .get("flasks")
        .and_then(Value::as_f64)
        .map(|n| n.floor().max(0.0) as i64)
        .unwrap_or(FLASK_BASE);
    push(&mut out, "flask", ScriptBindValue::Text(flasks.to_string()));
    push(&mut out, "flask.empty", ScriptBindValue::Flag(flasks < 1));

    let moss = item(save, "moss_potion");
    push(&mut out, "moss", ScriptBindValue::Number(moss as f64));
    push(&mut out, "moss.empty", ScriptBindValue::Flag(moss < 1));

    let prog = &save.progression;
    let xp = if prog.xp_next > 0 {
        prog.xp as f64 / prog.xp_next as f64
    } else {
        0.0
    };
    push(&mut out, "xp", ScriptBindValue::Number(xp));
    push(
        &mut out,
        "xp.text",
        ScriptBindValue::Text(format!("{}/{}", prog.xp, prog.xp_next)),
    );
    push(
        &mut out,
        "level",
        ScriptBindValue::Number(prog.level as f64),
    );
    push(
        &mut out,
        "level.text",
        ScriptBindValue::Text(prog.level.to_string()),
    );
    push(
        &mut out,
        "talent.points",
        ScriptBindValue::Number(prog.skill_points as f64),
    );
    push(
        &mut out,
        "talent.ready",
        ScriptBindValue::Flag(prog.skill_points > 0),
    );

    for (name, id) in [
        ("gold", "gold"),
        ("wood", "wood"),
        ("stone", "stone"),
        ("berry", "berry"),
        ("korok-seed", "korok-seed"),
        ("potion", "potion"),
        ("antidote", "antidote"),
        ("bomb", "bomb"),
    ] {
        push(
            &mut out,
            name,
            ScriptBindValue::Number(item(save, id) as f64),
        );
    }
    for (name, id) in [
        ("potion.empty", "potion"),
        ("antidote.empty", "antidote"),
        ("bomb.empty", "bomb"),
    ] {
        push(&mut out, name, ScriptBindValue::Flag(item(save, id) < 1));
    }

    let (title, progress, text) = best_quest(save, content);
    push(
        &mut out,
        "quest.title",
        ScriptBindValue::Text(title.clone()),
    );
    push(&mut out, "quest", ScriptBindValue::Number(progress));
    push(&mut out, "quest.text", ScriptBindValue::Text(text));
    push(
        &mut out,
        "quest.active",
        ScriptBindValue::Flag(!title.is_empty()),
    );
    // Outras missões (a coluna do `hud.xml`): o `quests.others` que o
    // `hud.lua` recalculava a cada 0.5 s. Verdadeiro quando há pelo menos
    // uma missão fora do cartão.
    push(
        &mut out,
        "quests.others",
        ScriptBindValue::Flag(!other_quest_rows(save, content, &title).is_empty()),
    );

    // Defaults do chefe: existem sem o Lua (o `lib/fsm.lua` escreve por cima
    // enquanto há chefe; a TTL em `hud_ui` põe `boss.active` a falso quando o
    // sinal pára). Constantes — só entram no primeiro publish.
    push(&mut out, "boss.active", ScriptBindValue::Flag(false));
    push(&mut out, "boss.name", ScriptBindValue::Text(String::new()));
    push(&mut out, "boss", ScriptBindValue::Number(1.0));
    push(&mut out, "boss.enraged", ScriptBindValue::Flag(false));

    out
}

fn push(out: &mut Vec<(String, ScriptBindValue)>, name: &str, value: ScriptBindValue) {
    out.push((name.to_string(), value));
}

/// `math.floor(v + 0.5)` do `rules.publish` (`%d` arredonda assim).
fn arredonda(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}

/// `num(chave, 0)` do `rules.lua`: só números contam (uma string não é
/// progresso).
fn num(save: &VaelSave, key: &str) -> f64 {
    save.legacy.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// `tonumber(...)` do Lua: número ou string numérica.
fn number_like(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite())
}

/// `held()`/`vault_get()` do `rules.lua`: nunca negativo.
fn item(save: &VaelSave, id: &str) -> i64 {
    save.inventory.get(id).max(0)
}

/// Objetivo em curso: a etapa actual (`q.<id>.stage`, 1-based e clampada) ou
/// o objetivo único — o `stage_index`/`objective` do `rules.lua`.
fn objective<'a>(save: &VaelSave, def: &'a QuestDef) -> &'a Objective {
    if def.stages.is_empty() {
        return &def.objective;
    }
    let at = save
        .legacy
        .get(&format!("q.{}.stage", def.id))
        .and_then(Value::as_f64)
        .map(|n| n.floor() as i64)
        .unwrap_or(1)
        .clamp(1, def.stages.len() as i64);
    &def.stages[(at - 1) as usize]
}

/// Meta efetiva: `q.<id>.need` sobrepõe o `count` do objetivo (como
/// `need_of`); um `count` a 0/ausente vale 1.
fn quest_need(save: &VaelSave, def: &QuestDef) -> f64 {
    if let Some(override_n) = save
        .legacy
        .get(&format!("q.{}.need", def.id))
        .and_then(number_like)
        && override_n >= 1.0
    {
        return override_n.floor();
    }
    (objective(save, def).count as f64).max(1.0).floor()
}

/// Progresso guardado (`q.<id>.n`).
fn quest_progress(save: &VaelSave, id: &str) -> f64 {
    num(save, &format!("q.{id}.n"))
}

/// O cartão do HUD: a missão seguida (`tracked_quest`) ganha; sem ela, a de
/// maior progresso (pronta > em curso > mais avançada), como `rules.publish`.
pub(crate) fn best_quest(save: &VaelSave, content: &VaelContent) -> (String, f64, String) {
    let tracked = save.quests.tracked.as_deref();
    let mut best_title = String::new();
    let mut best_progress = 0.0;
    let mut best_text = String::new();
    let mut best_score = f64::NEG_INFINITY;

    for (id, def) in &content.quests {
        // `quest_state`: um estado desconhecido conta como `not_taken`.
        let state = save
            .quests
            .states
            .get(id)
            .map(String::as_str)
            .unwrap_or("not_taken");
        if state != "active" && state != "ready" {
            continue;
        }
        let need = quest_need(save, def);
        let have = quest_progress(save, id);
        let mut score = if state == "ready" { 2.0 } else { 0.0 };
        if need > 0.0 {
            score += have / need;
        }
        if tracked == Some(id.as_str()) {
            // A missão seguida ganha sempre.
            score = 1e9;
        }
        if score <= best_score {
            continue;
        }
        best_score = score;
        best_title = if def.title.is_empty() {
            id.clone()
        } else {
            def.title.clone()
        };
        if state == "ready" {
            best_progress = 1.0;
            best_text = "pronta".to_string();
        } else {
            best_progress = (have / need).min(1.0);
            best_text = format!("{}/{}", have.floor() as i64, need.floor() as i64);
        }
    }

    (best_title, best_progress, best_text)
}

/// Linhas visíveis de "outras missões" por baixo do cartão (`OTHER_ROWS` do
/// `scripts/ui/hud.lua`).
pub const OTHER_ROWS: usize = 3;

/// Uma linha de "outras missões" (título já resolvido, sem a do cartão).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherQuestRow {
    pub title: String,
    pub ready: bool,
}

impl OtherQuestRow {
    /// Texto da linha: `"<título>  · pronta"` quando está pronta.
    pub fn text(&self) -> String {
        if self.ready {
            format!("{}  · pronta", self.title)
        } else {
            self.title.clone()
        }
    }
}

/// Missões em curso/prontas por baixo do cartão, sem a que está no cartão.
///
/// Mesma ordem do `refresh_other_quests` do Lua: prontas primeiro (são as que
/// pedem uma ação — voltar ao dador) e depois por título.
pub fn other_quest_rows(save: &VaelSave, content: &VaelContent, shown: &str) -> Vec<OtherQuestRow> {
    let mut rows: Vec<OtherQuestRow> = content
        .quests
        .iter()
        .filter_map(|(id, def)| {
            // `quest_state`: um estado desconhecido conta como `not_taken`.
            let state = save
                .quests
                .states
                .get(id)
                .map(String::as_str)
                .unwrap_or("not_taken");
            if state != "active" && state != "ready" {
                return None;
            }
            let title = if def.title.is_empty() {
                id.clone()
            } else {
                def.title.clone()
            };
            if title == shown {
                return None;
            }
            Some(OtherQuestRow {
                title,
                ready: state == "ready",
            })
        })
        .collect();
    // `b.ready.cmp(&a.ready)`: `true` fica à frente.
    rows.sort_by(|a, b| b.ready.cmp(&a.ready).then_with(|| a.title.cmp(&b.title)));
    rows
}

/// `"+N no diário [Q]"` (vazio quando não sobra nenhuma).
pub fn others_more_text(extra: usize) -> String {
    if extra == 0 {
        String::new()
    } else {
        format!("+{extra} no diário [Q]")
    }
}

/// Compara um valor Rust com o do Lua: texto exato para rótulos e número com
/// tolerância para barras (o `f32` do schema perde bits face ao `f64` do Lua).
fn same_bind(a: &ScriptBindValue, b: &ScriptBindValue) -> bool {
    match (a, b) {
        (ScriptBindValue::Text(x), ScriptBindValue::Text(y)) => x == y,
        (ScriptBindValue::Text(x), other) | (other, ScriptBindValue::Text(x)) => {
            x.as_str() == other.text()
        }
        _ => (a.number() - b.number()).abs() <= 1e-4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::VaelContent;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn kv_base() -> BTreeMap<String, Value> {
        let mut kv = BTreeMap::new();
        kv.insert("hp".into(), json!(73));
        kv.insert("max_hp".into(), json!(100));
        kv.insert("xp".into(), json!(50));
        kv.insert("xp_next".into(), json!(100));
        kv.insert("level".into(), json!(4));
        kv.insert("item.gold".into(), json!(12));
        kv.insert("q.wolf".into(), json!("active"));
        kv.insert("tracked_quest".into(), json!("wolf"));
        kv
    }

    fn conteudo() -> VaelContent {
        let mut quests = BTreeMap::new();
        quests.insert(
            "wolf".to_string(),
            QuestDef {
                id: "wolf".into(),
                title: "O Lobo".into(),
                objective: Objective {
                    objective_type: "kill".into(),
                    target: "wolf".into(),
                    count: 4,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        VaelContent {
            quests,
            ..Default::default()
        }
    }

    fn valor<'a>(binds: &'a [(String, ScriptBindValue)], name: &str) -> &'a ScriptBindValue {
        &binds
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("bind `{name}` ausente"))
            .1
    }

    fn texto(binds: &[(String, ScriptBindValue)], name: &str) -> String {
        valor(binds, name).text()
    }

    fn numero(binds: &[(String, ScriptBindValue)], name: &str) -> f64 {
        valor(binds, name).number()
    }

    fn flag(binds: &[(String, ScriptBindValue)], name: &str) -> bool {
        valor(binds, name).flag()
    }

    #[test]
    fn publica_vida_xp_e_ouro() {
        let save = VaelSave::from_world_kv(&kv_base());
        let binds = publish_binds(&save, &conteudo());

        // Barra = Number exato (a divisão 73/100 em f64 é a mesma que o Lua).
        assert!(matches!(
            valor(&binds, "health"),
            ScriptBindValue::Number(n) if *n == 0.73
        ));
        assert_eq!(texto(&binds, "health.text"), "73/100");
        assert_eq!(numero(&binds, "health.value"), 73.0);
        assert!(!flag(&binds, "health.low"));

        assert_eq!(numero(&binds, "xp"), 0.5);
        assert_eq!(texto(&binds, "xp.text"), "50/100");
        assert_eq!(numero(&binds, "level"), 4.0);
        assert_eq!(texto(&binds, "level.text"), "4");
        assert_eq!(numero(&binds, "talent.points"), 0.0);
        assert!(!flag(&binds, "talent.ready"));

        // Quantidades seguem o Lua: Number (o `UiText` lê `.text()`).
        assert!(matches!(
            valor(&binds, "gold"),
            ScriptBindValue::Number(n) if *n == 12.0
        ));
        assert_eq!(texto(&binds, "gold"), "12");
    }

    #[test]
    fn publica_quest_seguida() {
        let save = VaelSave::from_world_kv(&kv_base());
        let binds = publish_binds(&save, &conteudo());

        assert_eq!(texto(&binds, "quest.title"), "O Lobo");
        assert_eq!(numero(&binds, "quest"), 0.0);
        assert_eq!(texto(&binds, "quest.text"), "0/4");
        assert!(flag(&binds, "quest.active"));
    }

    #[test]
    fn publica_todos_os_binds_do_hud() {
        let save = VaelSave::from_world_kv(&kv_base());
        let binds = publish_binds(&save, &conteudo());
        for name in [
            "health",
            "health.text",
            "health.value",
            "health.low",
            "xp",
            "xp.text",
            "level",
            "level.text",
            "talent.points",
            "talent.ready",
            "gold",
            "wood",
            "stone",
            "berry",
            "korok-seed",
            "potion",
            "antidote",
            "bomb",
            "moss",
            "flask",
            "potion.empty",
            "antidote.empty",
            "bomb.empty",
            "moss.empty",
            "status.venom",
            "status.burn",
            "status.frost",
            "quest",
            "quest.text",
            "quest.title",
            "quest.active",
            "quests.others",
            "boss.active",
            "boss.name",
            "boss",
            "boss.enraged",
        ] {
            assert!(
                binds.iter().any(|(n, _)| n == name),
                "bind `{name}` não publicado"
            );
        }
    }

    #[test]
    fn guardas_de_zero_nao_dividem() {
        // hp/max_hp e xp/xp_next a zero: as barras ficam a 0, sem NaN/inf.
        let mut kv = BTreeMap::new();
        kv.insert("hp".into(), json!(0));
        kv.insert("max_hp".into(), json!(0));
        kv.insert("xp".into(), json!(0));
        kv.insert("xp_next".into(), json!(0));
        let save = VaelSave::from_world_kv(&kv);
        let binds = publish_binds(&save, &VaelContent::default());

        assert!(matches!(
            valor(&binds, "health"),
            ScriptBindValue::Number(n) if *n == 0.0
        ));
        assert!(matches!(
            valor(&binds, "xp"),
            ScriptBindValue::Number(n) if *n == 0.0
        ));
        assert_eq!(texto(&binds, "health.text"), "0/0");
        assert_eq!(texto(&binds, "xp.text"), "0/0");
        assert!(!flag(&binds, "quest.active"));
    }

    #[test]
    fn comparacao_aceita_texto_e_numero_equivalentes() {
        assert!(same_bind(
            &ScriptBindValue::Number(12.0),
            &ScriptBindValue::Text("12".into())
        ));
        assert!(same_bind(
            &ScriptBindValue::Number(0.73),
            &ScriptBindValue::Number(0.730_000_019)
        ));
        assert!(!same_bind(
            &ScriptBindValue::Flag(false),
            &ScriptBindValue::Flag(true)
        ));
        assert!(!same_bind(
            &ScriptBindValue::Text("pronta".into()),
            &ScriptBindValue::Number(1.0)
        ));
    }

    #[test]
    fn outras_quests_prontas_primeiro_sem_o_cartao() {
        // 4 em curso + 1 pronta; a seguida (a do cartão) fica de fora.
        let mut quests = BTreeMap::new();
        for (id, title) in [
            ("a", "Alfa"),
            ("b", "Beta"),
            ("c", "Gama"),
            ("d", "Delta"),
            ("e", "Épsilon"),
        ] {
            quests.insert(
                id.to_string(),
                QuestDef {
                    id: id.into(),
                    title: title.into(),
                    ..Default::default()
                },
            );
        }
        let content = VaelContent {
            quests,
            ..Default::default()
        };

        let mut kv = BTreeMap::new();
        for id in ["a", "b", "c", "d"] {
            kv.insert(format!("q.{id}"), json!("active"));
        }
        kv.insert("q.e".into(), json!("ready"));
        kv.insert("tracked_quest".into(), json!("a"));
        let save = VaelSave::from_world_kv(&kv);

        // O cartão é a missão seguida, como no `publish_binds`.
        let card = best_quest(&save, &content).0;
        assert_eq!(card, "Alfa");

        let rows = other_quest_rows(&save, &content, &card);
        // A do cartão sai; as outras quatro ficam, com a pronta à frente.
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows[0],
            OtherQuestRow {
                title: "Épsilon".into(),
                ready: true
            }
        );
        assert_eq!(rows[0].text(), "Épsilon  · pronta");
        assert!(rows.iter().all(|row| row.title != "Alfa"));
        // Sem prontas, o desempate é o título (bytes, como o `<` do Lua).
        let ativos: Vec<&str> = rows[1..].iter().map(|row| row.title.as_str()).collect();
        assert_eq!(ativos, ["Beta", "Delta", "Gama"]);
        // Só três cabem por baixo do cartão; a quarta vira o "+1".
        assert_eq!(
            others_more_text(rows.len().saturating_sub(OTHER_ROWS)),
            "+1 no diário [Q]"
        );
        assert_eq!(others_more_text(0), "");
    }

    #[test]
    fn publica_quests_others_e_defaults_do_chefe() {
        let save = VaelSave::from_world_kv(&kv_base());
        let binds = publish_binds(&save, &conteudo());

        // O cartão ("O Lobo") é a única missão em curso: não há outras.
        assert!(!flag(&binds, "quests.others"));
        assert!(!flag(&binds, "boss.active"));
        assert_eq!(texto(&binds, "boss.name"), "");
        assert_eq!(numero(&binds, "boss"), 1.0);
        assert!(!flag(&binds, "boss.enraged"));
    }

    #[test]
    fn plugins_correm_sem_host_nem_ui() {
        use crate::components::{Inventory, PlayerVitals, QuestLog, XpState};
        use bevy::time::Real;

        // App mínima: sem `LuaScriptHost` e sem `UiScriptBinds`. Os dois
        // sistemas têm de correr sem pânico e deixar o estado intacto.
        let mut app = App::new();
        app.init_resource::<Time<Real>>()
            .init_resource::<PlayerVitals>()
            .init_resource::<XpState>()
            .init_resource::<Inventory>()
            .init_resource::<QuestLog>()
            .init_resource::<VaelSave>()
            .add_plugins((crate::state_sync::VaelStateSyncPlugin, VaelHudPlugin));

        for _ in 0..3 {
            app.update();
        }
        assert_eq!(app.world().resource::<VaelSave>(), &VaelSave::default());
    }
}
