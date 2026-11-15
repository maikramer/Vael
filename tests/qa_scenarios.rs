//! Gate dos cenários de QA: cada `qa/scenarios/**/*.json` (menos os
//! `_base`/`_*`) carrega, passa o lint (ids de quests/itens/talentos/cena) e
//! a simulação dos passos lógicos com as regras reais do Vael chega ao
//! `expect`. Sem janela, sem GPU: corre em milissegundos.
//!
//! Um cenário que falhe aqui mostra o relatório completo (passos e falhas),
//! o mesmo que `tools/qa.py sim <nome>`.

use std::path::Path;

use vael::content::VaelContent;
use vael::qa::{QaRules, lint, list_scenarios, load_scenario, simulate};

#[test]
fn todos_os_cenarios_passam_na_simulacao() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let content = VaelContent::load(root);
    assert!(
        content.errors.is_empty(),
        "conteúdo inválido: {:?}",
        content.errors
    );
    let rules = QaRules::from_content(&content);

    let names: Vec<String> = list_scenarios(root)
        .into_iter()
        .filter(|name| !name.rsplit('/').next().unwrap_or(name).starts_with('_'))
        .collect();
    let mut failed = Vec::new();
    for name in &names {
        let loaded = match load_scenario(root, name) {
            Ok(loaded) => loaded,
            Err(err) => {
                failed.push(format!("{name}: {err}"));
                continue;
            }
        };
        let problems = lint(root, &rules, &loaded.scenario);
        if !problems.is_empty() {
            failed.push(format!("{name}: lint: {problems:?}"));
        }
        let report = simulate(&rules, name, &loaded.scenario);
        if !report.ok {
            failed.push(format!(
                "{name}: {}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "{} cenário(s) a falhar:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// Os bases (`_base`, `_*`) também têm de ser JSON válido do schema.
#[test]
fn bases_carregam() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in list_scenarios(root) {
        if name.rsplit('/').next().unwrap_or(&name).starts_with('_') {
            load_scenario(root, &name).unwrap_or_else(|err| panic!("{name}: {err}"));
        }
    }
}
