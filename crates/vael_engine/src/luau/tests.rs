//! Testes que sobreviveram ao host antigo (2026-11-15, onda final do
//! DELETE-MONOLITH): o fuzzy match de gestos e o lookup de clips vivem no
//! `bevy_ext_engine::luau` e não precisam de VM — ficam aqui com o
//! consumidor da engine. TODOS os testes do HOST (chunks, timers, sinais,
//! eventos, vitais, game/JSON, docs da API) morreram com ele; os que a
//! superfície BMS precisa vivem nos testes do `bevy_ext_luau` (F9: os
//! restantes Parity tests seguem para lá, um por comando).

use super::*;
use bevy::prelude::*;

/// CharacterAnimator mínimo com os clips dados (1 s cada, nós 1..n).
fn test_animator(names: &[&str]) -> crate::animation::CharacterAnimator {
    crate::animation::CharacterAnimator::new(
        names.iter().map(|s| s.to_string()).collect(),
        (0..names.len())
            .map(|i| bevy::animation::graph::AnimationNodeIndex::new(i + 1))
            .collect(),
        vec![1.0; names.len()],
        Entity::PLACEHOLDER,
        None,
    )
}

#[test]
fn test_match_gesture_clip_fuzzy_and_alternatives() {
    // Pack npc_* real: prefixos de ferramenta, `_` e caixa variam.
    let a = test_animator(&["Animator3D_Talk", "Animator3D_Yes", "npc_Fold_Arms", "idle"]);
    assert_eq!(match_gesture_clip(&a, "yes"), Some(1));
    assert_eq!(match_gesture_clip(&a, "YES"), Some(1), "case-insensitive");
    assert_eq!(match_gesture_clip(&a, "talk"), Some(0));
    // "foldarms" encontra "npc_Fold_Arms" (normalização de ambos os lados).
    assert_eq!(match_gesture_clip(&a, "foldarms"), Some(2));
    assert_eq!(match_gesture_clip(&a, "fold_arms"), Some(2));
    // Clip inexistente = None; a alternativa seguinte salva.
    assert_eq!(match_gesture_clip(&a, "salute"), None);
    assert_eq!(match_gesture_clip(&a, "salute,yes"), Some(1));
    assert_eq!(match_gesture_clip(&a, "salute|talk"), Some(0));
    // Pedido vazio (ou só separadores) nunca dá match.
    assert_eq!(match_gesture_clip(&a, ""), None);
    assert_eq!(match_gesture_clip(&a, ","), None);
}

#[test]
fn test_sfx_clip_from_str_maps_known_names() {
    assert_eq!(sfx_clip_from_str("hit"), Some(crate::ambient::SfxClip::Hit));
    assert_eq!(
        sfx_clip_from_str("UI"),
        Some(crate::ambient::SfxClip::Ui),
        "case-insensitive"
    );
    assert_eq!(
        sfx_clip_from_str("harvest"),
        Some(crate::ambient::SfxClip::Harvest)
    );
    assert_eq!(
        sfx_clip_from_str("whoosh"),
        Some(crate::ambient::SfxClip::Whoosh)
    );
    assert_eq!(sfx_clip_from_str("boom"), None);
}



