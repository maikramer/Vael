//! Lint estrutural do mundo. A primeira versão reutiliza o `analyze` e
//! classifica os problemas; as regras de scripts/cinematics do lint antigo
//! entram por cima sem mudar o formato `Finding`.

use std::path::Path;

use serde::Serialize;

use crate::analyze::{analyze, resolve_world_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub kind: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LintReport {
    pub findings: Vec<Finding>,
}

impl LintReport {
    pub fn errors(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == Severity::Error)
            .count()
    }

    pub fn warnings(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == Severity::Warning)
            .count()
    }

    pub fn has_errors(&self) -> bool {
        self.errors() > 0
    }
}

/// Lint de um mundo. `strict` promove warnings a falha no CLI, não aqui.
pub fn lint(input: Option<&Path>) -> LintReport {
    let report = analyze(input);
    let mut findings = Vec::new();
    if let Some(error) = report.error {
        findings.push(Finding {
            severity: Severity::Error,
            kind: "world",
            message: error,
        });
        return LintReport { findings };
    }
    for missing in report.assets.missing {
        findings.push(Finding {
            severity: Severity::Warning,
            kind: "asset",
            message: format!("asset em falta: {missing}"),
        });
    }
    // Root = a pasta do mundo RESOLVIDO (o input cru pode ser uma pasta ou
    // outro ficheiro; `scripts/` vive ao lado do mundo, como no `analyze`).
    let world = resolve_world_path(input).ok();
    let root = world
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."));
    for script in report.scripts {
        let rel = script.trim_start_matches('/');
        if !root.join("scripts").join(rel).is_file() {
            findings.push(Finding {
                severity: Severity::Error,
                kind: "script",
                message: format!("script em falta: scripts/{rel}"),
            });
        }
    }
    for warning in report.warnings {
        // O `analyze` já avisa dos scripts em falta; o lint promove-os a erro
        // acima. Repetir o aviso duplicava o finding e inflava a contagem.
        if warning.starts_with("script em falta: ") {
            continue;
        }
        findings.push(Finding {
            severity: Severity::Warning,
            kind: "world",
            message: warning,
        });
    }
    LintReport { findings }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_script_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("world.xml"),
            r#"<World><Entity name="npc" script="missing.lua" /></World>"#,
        )
        .unwrap();
        let report = lint(Some(&dir.path().join("world.xml")));
        assert!(report.has_errors());
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == "script")
        );
        // Regressão: o aviso do `analyze` repetia o mesmo script em falta
        // como finding de "world" (2 avisos + 1 erro para o mesmo problema).
        assert_eq!(report.errors(), 1);
        assert_eq!(report.warnings(), 0);
    }
}
