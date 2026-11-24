//! Instalação de `vael.vael` nos contextos BMS **e na REPL do bridge**.
//!
//! O host antigo tinha UMA VM partilhada: os plugins do jogo (cofre, XP,
//! economia, …) instalavam os seus closures Rust nela uma vez, no arranque.
//! O host BMS cria um contexto POR ENTIDADE (com o attach do script), e os
//! contextos nascem à medida que as entidades com `LuaScriptRef` entram no
//! mundo — não há "a VM" para instalar uma vez.
//!
//! A ponte é um registo LOCAL ([`register_installer`] — closures com capturas:
//! rules derivadas do `VaelContent`, filas `Arc`) mais UM hook de fn pointer
//! publicado no `bevy_ext_luau` por [`install_context_hook`]. O
//! `install_viber_api` do crate corre o hook por contexto novo, DEPOIS da
//! tabela `viber`, e a REPL dedicada do bridge (`vael.lua`) corre o MESMO
//! hook — `vael.vael.qa_snapshot`/`dodge`/`attack_motion` existem lá como num
//! script de jogo (era o que faltava: o registo local do Vael não chegava à
//! REPL, `debug state` morria em `nil.qa_snapshot`). Um hook por processo
//! (`Once`): apps de teste que constroem vários `App` não empilham hooks.
//!
//! [`run_installers`] mantém-se para os testes que instalam numa Lua nua (o
//! registo local guarda os mesmos closures).

use std::sync::{OnceLock, RwLock};

use mlua::Lua;

type Installer = Box<dyn Fn(&Lua) -> mlua::Result<()> + Send + Sync>;

fn registry() -> &'static RwLock<Vec<Installer>> {
    static REGISTRY: OnceLock<RwLock<Vec<Installer>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Regista um instalador `vael.vael` (corre em CADA contexto novo E na REPL).
///
/// Chamado pelos plugins do jogo no arranque, com os dados capturados
/// (`Arc` de rules, filas partilhadas). A ordem de registo é a ordem de
/// execução dos sistemas que o chamam — as cadeias `.after()` de Startup
/// preservam o mesmo encadeamento do host antigo.
///
/// O registo é LOCAL (closures com capturas não cabem no registo de fn
/// pointers do `bevy_ext_luau`); [`install_context_hook`] publica UM hook
/// no crate que corre este registo tanto nos contextos BMS como na REPL.
pub fn register_installer(installer: impl Fn(&Lua) -> mlua::Result<()> + Send + Sync + 'static) {
    if let Ok(mut installers) = registry().write() {
        installers.push(Box::new(installer));
    }
}

/// Publica no `bevy_ext_luau` o hook que corre os instaladores locais (uma
/// vez por processo — `Once`; apps de teste que constroem vários `App`
/// re-registam sem duplicar). O crate corre-o por contexto BMS e na REPL do
/// bridge (`vael.lua`), depois da tabela `viber`.
pub fn install_context_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| bevy_ext_luau::add_context_installer(context_hook));
}

/// Hook sem capturas: lê o registo LOCAL vivo (os plugins registam nos
/// sistemas de Startup) e corre-o na Lua do contexto/REPL.
fn context_hook(lua: &Lua) -> Result<(), String> {
    run_installers(lua).map_err(|err| err.to_string())
}

/// Corre todos os instaladores registados numa VM/contexto. Um instalador
/// que falha NUNCA falha o contexto (o erro é do closure do módulo, não do
/// script — os restantes módulos e o próprio script continuam; o erro fica
/// no log uma vez por contexto).
///
/// Usado pelos testes que instalam numa Lua nua; os contextos do jogo correm
/// pelo registo do `bevy_ext_luau` (o `install_viber_api` do crate).
pub fn run_installers(lua: &Lua) -> mlua::Result<()> {
    let failed = registry()
        .read()
        .map(|installers| {
            for installer in installers.iter() {
                if let Err(err) = installer(lua) {
                    eprintln!("vael: falha a instalar `vael.vael` num contexto: {err}");
                }
            }
            false
        })
        .unwrap_or(true);
    if failed {
        eprintln!(
            "vael: registo de instaladores fechado a erro — contextos nascem sem `vael.vael`"
        );
    }
    Ok(())
}
