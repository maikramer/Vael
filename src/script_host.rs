//! Instalação de `viber.vael` nos contextos BMS.
//!
//! O host antigo tinha UMA VM partilhada: os plugins do jogo (cofre, XP,
//! economia, …) instalavam os seus closures Rust nela uma vez, no arranque.
//! O host BMS cria um contexto POR ENTIDADE (com o attach do script), e os
//! contextos nascem à medida que as entidades com `LuaScriptRef` entram no
//! mundo — não há "a VM" para instalar uma vez.
//!
//! A ponte é um REGISTO: cada plugin regista o seu instalador no arranque
//! ([`register_installer`], com os mesmos dados de sempre — rules derivadas
//! do `VaelContent`, filas `Arc` partilhadas) e o [`context_initializer`]
//! passado ao `VaelScriptingPlugin::extra_initializers` corre todos por
//! contexto novo, na ordem de registo (a ordem dos sistemas de Startup que
//! os plugins já encadeavam — `.after(crate::vault::install_vault_api)`).

use std::sync::{OnceLock, RwLock};

use bevy_ext_luau::{ContextInitializer, InteropError, LuaContext, LuaScriptingPlugin};
use mlua::Lua;

type Installer = Box<dyn Fn(&Lua) -> mlua::Result<()> + Send + Sync>;

fn registry() -> &'static RwLock<Vec<Installer>> {
    static REGISTRY: OnceLock<RwLock<Vec<Installer>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Regista um instalador `viber.vael` (corre em CADA contexto novo).
///
/// Chamado pelos plugins do jogo no arranque, com os dados capturados
/// (`Arc` de rules, filas partilhadas). A ordem de registo é a ordem de
/// execução dos sistemas que o chamam — as cadeias `.after()` de Startup
/// preservam o mesmo encadeamento do host antigo.
pub fn register_installer(installer: impl Fn(&Lua) -> mlua::Result<()> + Send + Sync + 'static) {
    if let Ok(mut installers) = registry().write() {
        installers.push(Box::new(installer));
    }
}

/// Corre todos os instaladores registados numa VM/contexto. Um instalador
/// que falha NUNCA falha o contexto (o erro é do closure do módulo, não do
/// script — os restantes módulos e o próprio script continuam; o erro fica
/// no log uma vez por contexto).
pub fn run_installers(lua: &Lua) -> mlua::Result<()> {
    let failed = registry()
        .read()
        .map(|installers| {
            for installer in installers.iter() {
                if let Err(err) = installer(lua) {
                    eprintln!("vael: falha a instalar `viber.vael` num contexto: {err}");
                }
            }
            false
        })
        .unwrap_or(true);
    if failed {
        eprintln!("vael: registo de instaladores fechado a erro — contextos nascem sem `viber.vael`");
    }
    Ok(())
}

/// O inicializador de contexto para o `VaelScriptingPlugin`: corre os
/// instaladores do jogo DEPOIS do `install_viber_api` do crate (a ordem dos
/// `context_initializers` é a de registo no plugin).
pub fn context_initializer(
    _attachment: &bevy_ext_luau::ScriptAttachment,
    context: &mut LuaContext,
) -> Result<(), InteropError> {
    run_installers(&**context).map_err(InteropError::external)
}

/// O conjunto pronto a passar ao plugin (`extra_initializers` do
/// `bevy_ext_luau::VaelScriptingPlugin`).
pub fn extra_initializers() -> Vec<ContextInitializer<LuaScriptingPlugin>> {
    vec![context_initializer]
}
