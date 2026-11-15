//! Ferramentas headless do mundo do Vael: `analyze`, `lint` e `bsn`.
//!
//! Não dependem do monólito nem de uma App Bevy: leem o mundo pelo
//! `bevy_ext_world` (que preserva `WorldNode`/`WorldProp` e os `TypePath` de
//! wire). A lib alimenta o binário `vael-tools` e a extensão
//! `vael_analyze_jackdaw` (painel Problems do Jackdaw).

pub mod analyze;
pub mod bsn;
pub mod lint;

pub use analyze::{AnalyzeReport, analyze, resolve_world_path};
pub use bsn::{check as check_bsn, convert as convert_bsn};
pub use lint::{Finding, LintReport, Severity, lint};
