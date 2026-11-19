//! Transições persistentes do contrato de Edmar e do primeiro mapa.
use super::{
    KEY_TRACKED_QUEST, QuestKv, QuestRules, STATE_ACTIVE, STATE_DONE, STATE_NOT_TAKEN, STATE_READY,
    quest_accept, quest_objective, quest_set_progress, quest_turn_in, raw_state, raw_string,
};
use crate::vault::VaultRules;
use crate::xp::XpRules;
use serde_json::json;

/// A abertura termina na entrega do capítulo jogável. Saves anteriores e
/// cenários avulsos conservam as ofertas até iniciarem o novo contrato.
pub(crate) const FIRST_FOLD: &str = "story_first_fold";
pub(crate) const CONTRACT: &str = "story_notas";
pub(crate) const VALLEY_TOWER: &str = "atalaia-do-vale";
pub(crate) const REGIONAL_QUESTS: [&str; 4] = [
    "forest_witch",
    "desert_worm",
    "swamp_bogwarden",
    "peaks_ogre",
];

pub(super) fn opening_in_progress(kv: &QuestKv) -> bool {
    kv.get("campaign.opening").and_then(|v| v.as_bool()) == Some(true)
        && raw_state(kv, CONTRACT) != Some(STATE_DONE)
}

pub(super) fn opening_allows(kv: &QuestKv, id: &str) -> bool {
    if id == FIRST_FOLD {
        return matches!(raw_state(kv, CONTRACT), Some(STATE_ACTIVE | STATE_READY));
    }
    !opening_in_progress(kv)
        || id == CONTRACT
        || (raw_state(kv, FIRST_FOLD) == Some(STATE_DONE) && REGIONAL_QUESTS.contains(&id))
}

/// O alvo escolhido no contrato permanece o destino mesmo antes de falar ao dador.
pub(crate) fn opening_destination<'a>(kv: &'a QuestKv) -> Option<&'a str> {
    if !opening_in_progress(kv)
        || raw_state(kv, FIRST_FOLD) != Some(STATE_DONE)
        || raw_state(kv, CONTRACT) == Some(STATE_READY)
    {
        return None;
    }
    let id = raw_string(kv, "campaign.region")?;
    (REGIONAL_QUESTS.contains(&id) && raw_state(kv, id).is_none_or(|s| s == STATE_NOT_TAKEN))
        .then_some(id)
}

/// Cada comando só tem efeito na etapa que o originou. O chamador publica
/// o diff inteiro depois do sucesso, incluindo a escolha e a recompensa.
pub(super) fn apply(
    rules: &QuestRules,
    vault: &VaultRules,
    xp: &XpRules,
    kv: &mut QuestKv,
    action: &str,
    value: &str,
) -> bool {
    match action {
        "contract" => {
            if !REGIONAL_QUESTS.contains(&value) || !quest_accept(rules, kv, CONTRACT) {
                return false;
            }
            kv.insert("campaign.opening".into(), json!(true));
            kv.insert("campaign.region".into(), json!(value));
            if !quest_accept(rules, kv, FIRST_FOLD) {
                return false;
            }
            kv.insert(KEY_TRACKED_QUEST.into(), json!(FIRST_FOLD));
            if kv.get("mark.atalaia-do-vale") == Some(&json!(true)) {
                quest_set_progress(rules, kv, FIRST_FOLD, 1.0);
            }
            true
        }
        "sign" | "draw" | "arrive" => {
            if raw_state(kv, FIRST_FOLD) != Some(STATE_ACTIVE) {
                return false;
            }
            let step = quest_objective(rules, kv, FIRST_FOLD);
            match (action, step.target.as_str()) {
                ("sign", "fold_sign") if kv.get("mark.atalaia-do-vale") == Some(&json!(true)) => {
                    quest_set_progress(rules, kv, FIRST_FOLD, 1.0).ok
                }
                ("draw", "fold_draw") if matches!(value, "roof" | "wall") => {
                    kv.insert("campaign.fold".into(), json!(value));
                    quest_set_progress(rules, kv, FIRST_FOLD, 1.0).ok
                }
                ("arrive", "fold_arrive") => {
                    if !quest_set_progress(rules, kv, FIRST_FOLD, 1.0).ready {
                        return false;
                    }
                    quest_turn_in(rules, vault, xp, kv, FIRST_FOLD).ok
                }
                _ => false,
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::VaelContent;
    use crate::quest::{quest_available, quest_state};
    use crate::save::VaelSave;
    use std::path::Path;

    fn rules() -> (QuestRules, VaultRules, XpRules) {
        let content = VaelContent::load(Path::new(env!("CARGO_MANIFEST_DIR")));
        assert!(content.errors.is_empty(), "{:?}", content.errors);
        (
            QuestRules::from_content(&content),
            VaultRules::from_items(&content.items),
            XpRules::from_progression(&content.progression),
        )
    }

    #[test]
    fn contract_and_fold_resume_at_the_selected_giver() {
        let (rules, vault, xp) = rules();
        for region in REGIONAL_QUESTS {
            for legend in ["roof", "wall"] {
                let mut kv = QuestKv::from([("campaign.opening".into(), json!(true))]);
                let act =
                    |kv: &mut QuestKv, action, value| apply(&rules, &vault, &xp, kv, action, value);
                assert_eq!(
                    rules
                        .order
                        .iter()
                        .filter(|id| quest_available(&rules, &kv, id))
                        .collect::<Vec<_>>(),
                    vec![CONTRACT]
                );
                assert!(!act(&mut kv, "contract", "unknown"));
                assert!(act(&mut kv, "contract", region));
                assert!(!act(&mut kv, "contract", region));
                assert!(!quest_accept(&rules, &mut kv, "city_stone"));
                assert!(!quest_accept(&rules, &mut kv, region));
                assert!(!act(&mut kv, "sign", ""));
                kv.insert("mark.atalaia-do-vale".into(), json!(true));
                assert!(act(&mut kv, "sign", ""));
                assert!(!act(&mut kv, "arrive", ""));
                assert!(!act(&mut kv, "draw", "yard"));
                assert!(act(&mut kv, "draw", legend));
                let mut kv = VaelSave::from_world_kv(&kv).to_world_kv();
                assert_eq!(kv["campaign.fold"], json!(legend));
                assert_eq!(
                    quest_objective(&rules, &kv, FIRST_FOLD).target,
                    "fold_arrive"
                );
                assert!(act(&mut kv, "arrive", ""));
                let done = kv.clone();
                assert!(!act(&mut kv, "arrive", ""));
                assert_eq!(kv, done);
                assert_eq!(opening_destination(&kv), Some(region));
                assert!(quest_accept(&rules, &mut kv, region));
                assert_eq!(opening_destination(&kv), None);
                assert!(!quest_available(&rules, &kv, "city_stone"));
            }
        }
    }

    #[test]
    fn prior_signature_and_four_notes_skip_only_completed_work() {
        let (rules, vault, xp) = rules();
        let mut kv = QuestKv::from([
            ("mark.atalaia-do-vale".into(), json!(true)),
            ("item.nota".into(), json!(4)),
        ]);
        assert!(quest_available(&rules, &kv, "city_stone"));
        assert!(apply(
            &rules,
            &vault,
            &xp,
            &mut kv,
            "contract",
            "forest_witch"
        ));
        assert_eq!(quest_objective(&rules, &kv, FIRST_FOLD).target, "fold_draw");
        assert!(quest_set_progress(&rules, &mut kv, CONTRACT, 4.0).ready);
        assert!(!quest_turn_in(&rules, &vault, &xp, &mut kv, CONTRACT).ok);
        assert_eq!(kv["item.nota"], json!(4));
        assert!(apply(&rules, &vault, &xp, &mut kv, "draw", "wall"));
        assert!(apply(&rules, &vault, &xp, &mut kv, "arrive", ""));
        assert!(opening_destination(&kv).is_none());
        assert!(quest_turn_in(&rules, &vault, &xp, &mut kv, CONTRACT).ok);
        assert_eq!(quest_state(&rules, &kv, CONTRACT), STATE_DONE);
        assert!(quest_available(&rules, &kv, "city_stone"));
    }
}
