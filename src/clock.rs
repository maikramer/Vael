//! Relógio e chuva do mundo em Rust: a hora que o `lib/clock.lua` lia do HUD
//! passa a sair dos recursos da engine.
//!
//! [`ClockSnapshot`] é a resposta que o Lua vê (`hour` em horas decimais e
//! `rain` 0..1). [`VaelClock`] é o handle partilhado (`Arc<Mutex>`) que o
//! sistema Bevy escreve e as closures de `viber.vael` leem. Sem
//! `DayCycleState` (mundos QA) a hora é `None` e o Lua responde "não é de
//! noite", como antes; sem `WeatherState` a chuva é 0.
//!
//! Regras portadas de `scripts/lib/clock.lua`:
//! - noite = hora ≥ 20:00 ou < 05:00;
//! - chuva "a sério" (apaga o fogo, corta a visão) = ≥ 0,3.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mlua::{Lua, Table, Value};
use viber::luau::host::LuaScriptHost;
use viber::worldsys::{DayCycleState, WeatherState};

/// Hora em que a noite começa (20:00).
pub const NIGHT_FROM: f64 = 20.0;
/// Hora em que a noite acaba (05:00).
pub const NIGHT_TO: f64 = 5.0;
/// Chuva a partir da qual conta como pesada (`RAIN_HEAVY` do Lua).
pub const RAIN_HEAVY: f64 = 0.3;

/// Minutos de um dia completo.
const MINUTES_PER_DAY: f32 = 1440.0;

/// Hora e chuva do mundo tal como o Lua as vê.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockSnapshot {
    /// Hora decimal (`19.5` = 19:30); `None` sem `<DayCycle>`.
    pub hour: Option<f64>,
    /// Intensidade da chuva 0..1.
    pub rain: f64,
}

impl Default for ClockSnapshot {
    fn default() -> Self {
        Self {
            hour: None,
            rain: 0.0,
        }
    }
}

/// Handle partilhado do snapshot: o sistema escreve, a bridge lê.
#[derive(Resource, Debug, Clone, Default)]
pub struct VaelClock(pub Arc<Mutex<ClockSnapshot>>);

impl VaelClock {
    /// Cópia do snapshot actual (lock recuperado mesmo depois de poison).
    pub fn get(&self) -> ClockSnapshot {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Substitui o snapshot.
    pub fn set(&self, snapshot: ClockSnapshot) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = snapshot;
    }
}

/// Hora decimal de um minuto do dia.
///
/// A volta do dia é `rem_euclid`: 1439 min vale 23,98 e nunca 24,xx.
pub fn hour_from_minute(minute_of_day: f32) -> f64 {
    (minute_of_day.rem_euclid(MINUTES_PER_DAY) / 60.0) as f64
}

/// Snapshot a partir dos recursos da engine.
pub fn snapshot(
    day_cycle: Option<&DayCycleState>,
    weather: Option<&WeatherState>,
) -> ClockSnapshot {
    ClockSnapshot {
        hour: day_cycle.map(|clock| hour_from_minute(clock.minute_of_day)),
        rain: weather.map_or(0.0, |weather| weather.rain.clamp(0.0, 1.0) as f64),
    }
}

/// Hora de um snapshot (`M.hour` do Lua).
pub fn hour(snapshot: &ClockSnapshot) -> Option<f64> {
    snapshot.hour
}

/// Intensidade de chuva de um snapshot (`M.rain` do Lua), presa a 0..1.
pub fn rain(snapshot: &ClockSnapshot) -> f64 {
    snapshot.rain.clamp(0.0, 1.0)
}

/// `true` entre as 20:00 e as 05:00; sem relógio é dia (`M.is_night`).
pub fn is_night(hour: Option<f64>) -> bool {
    match hour {
        Some(hour) => hour >= NIGHT_FROM || hour < NIGHT_TO,
        None => false,
    }
}

/// `true` com chuva ≥ [`RAIN_HEAVY`] (`M.raining`).
pub fn raining(rain: f64) -> bool {
    rain >= RAIN_HEAVY
}

// ── sistemas ────────────────────────────────────────────────────────────

/// Copia `DayCycleState`/`WeatherState` para o snapshot partilhado.
///
/// Corre no `PreUpdate` para os scripts do `Update` verem o estado do frame.
fn update_clock_snapshot(
    clock: Res<VaelClock>,
    day_cycle: Option<Res<DayCycleState>>,
    weather: Option<Res<WeatherState>>,
) {
    let next = snapshot(day_cycle.as_deref(), weather.as_deref());
    clock.set(next);
}

// ── bridge Luau ─────────────────────────────────────────────────────────

/// Plugin do relógio: instala `viber.vael.clock_*` depois do combate.
pub struct VaelClockPlugin;

impl Plugin for VaelClockPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VaelClock>()
            .add_systems(PreUpdate, update_clock_snapshot)
            .add_systems(
                Startup,
                install_clock_api.after(crate::combat::install_combat_api),
            );
    }
}

/// Arranque: acrescenta as funções do relógio à tabela `viber.vael`.
pub(crate) fn install_clock_api(host: Option<Res<LuaScriptHost>>, clock: Res<VaelClock>) {
    let Some(host) = host else {
        warn!("vael: relógio sem host Luau — `viber.vael` não estendido");
        return;
    };
    match install(&host.lua, &clock) {
        Ok(()) => info!("vael: relógio em Rust ligado (`viber.vael`)"),
        Err(err) => error!("vael: falha a instalar relógio em `viber.vael`: {err}"),
    }
}

/// Acrescenta `clock_hour`, `clock_is_night`, `clock_rain` e `clock_raining`
/// a `viber.vael` sem substituir as funções que já lá estão.
pub(crate) fn install(lua: &Lua, clock: &VaelClock) -> mlua::Result<()> {
    let viber: Table = lua.globals().get("viber")?;
    let vael: Table = match viber.get::<Value>("vael")? {
        Value::Table(table) => table,
        _ => {
            let table = lua.create_table()?;
            viber.set("vael", table.clone())?;
            table
        }
    };

    {
        let clock = clock.clone();
        vael.set(
            "clock_hour",
            lua.create_function(move |_, ()| {
                Ok(match hour(&clock.get()) {
                    Some(hour) => Value::Number(hour),
                    None => Value::Nil,
                })
            })?,
        )?;
    }

    {
        let clock = clock.clone();
        vael.set(
            "clock_is_night",
            lua.create_function(move |_, ()| Ok(is_night(hour(&clock.get()))))?,
        )?;
    }

    {
        let clock = clock.clone();
        vael.set(
            "clock_rain",
            lua.create_function(move |_, ()| Ok(rain(&clock.get())))?,
        )?;
    }

    {
        let clock = clock.clone();
        vael.set(
            "clock_raining",
            lua.create_function(move |_, ()| Ok(raining(rain(&clock.get()))))?,
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Function;

    fn snap(hour: Option<f64>, rain: f64) -> ClockSnapshot {
        ClockSnapshot { hour, rain }
    }

    #[test]
    fn noite_comeca_as_20_e_acaba_as_5() {
        assert!(!is_night(Some(19.0 + 59.0 / 60.0)), "19:59 ainda é dia");
        assert!(is_night(Some(20.0)), "20:00 já é noite");
        assert!(is_night(Some(4.0 + 59.0 / 60.0)), "04:59 ainda é noite");
        assert!(!is_night(Some(5.0)), "05:00 é dia");
        assert!(!is_night(None), "sem relógio é dia");
    }

    #[test]
    fn chuva_pesada_a_partir_de_0_3() {
        assert!(!raining(0.29));
        assert!(raining(0.30));
        assert!(raining(1.0));
        assert!(!raining(0.0));
    }

    #[test]
    fn snapshot_faz_a_volta_do_dia() {
        // 1439 min = 23:59, não 24:xx.
        let hora = hour_from_minute(1439.0);
        assert!((hora - 23.98).abs() < 0.01, "hora = {hora}");
        assert!((hour_from_minute(1440.0) - 0.0).abs() < 1e-9);
        assert!((hour_from_minute(12.0 * 60.0 + 30.0) - 12.5).abs() < 1e-9);
    }

    #[test]
    fn snapshot_ausente_vira_none_e_chuva_zero() {
        let s = snapshot(None, None);
        assert_eq!(s, snap(None, 0.0));
        assert_eq!(hour(&s), None);
        assert_eq!(rain(&s), 0.0);
        assert!(!is_night(hour(&s)));
        assert!(!raining(rain(&s)));
    }

    #[test]
    fn snapshot_limita_a_chuva_a_0_1() {
        let weather = WeatherState {
            wind: [0.0, 0.0],
            wind_strength: 0.0,
            clouds: 0.0,
            rain: 2.5,
            cycle: false,
        };
        assert_eq!(snapshot(None, Some(&weather)).rain, 1.0);

        let negative = WeatherState {
            rain: -1.0,
            ..weather
        };
        assert_eq!(snapshot(None, Some(&negative)).rain, 0.0);
    }

    fn lua_com_bridge(clock: VaelClock) -> Lua {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        install(&lua, &clock).expect("instalar viber.vael");
        lua
    }

    #[test]
    fn bridge_le_o_snapshot_partilhado() {
        let clock = VaelClock::default();
        clock.set(snap(Some(21.5), 0.4));
        let lua = lua_com_bridge(clock);

        let hora: f64 = lua
            .load("return viber.vael.clock_hour()")
            .eval()
            .expect("clock_hour");
        assert_eq!(hora, 21.5);
        let noite: bool = lua
            .load("return viber.vael.clock_is_night()")
            .eval()
            .expect("clock_is_night");
        assert!(noite);
        let chuva: f64 = lua
            .load("return viber.vael.clock_rain()")
            .eval()
            .expect("clock_rain");
        assert_eq!(chuva, 0.4);
        let chove: bool = lua
            .load("return viber.vael.clock_raining()")
            .eval()
            .expect("clock_raining");
        assert!(chove);
    }

    #[test]
    fn bridge_sem_relogio_devolve_nil_e_dia() {
        let lua = lua_com_bridge(VaelClock::default());
        assert!(
            lua.load("return viber.vael.clock_hour() == nil")
                .eval::<bool>()
                .expect("clock_hour nil")
        );
        assert!(
            !lua.load("return viber.vael.clock_is_night()")
                .eval::<bool>()
                .expect("clock_is_night")
        );
        let chuva: f64 = lua
            .load("return viber.vael.clock_rain()")
            .eval()
            .expect("clock_rain");
        assert_eq!(chuva, 0.0);
    }

    #[test]
    fn modulo_lua_usa_a_bridge_ao_vivo() {
        let clock = VaelClock::default();
        clock.set(snap(Some(21.25), 0.2));
        let lua = lua_com_bridge(clock.clone());
        let module: Table = lua
            .load(include_str!("../scripts/lib/clock.lua"))
            .set_name("lib/clock.lua")
            .eval()
            .expect("carregar clock.lua");

        let hora: f64 = module
            .get::<Function>("hour")
            .expect("M.hour")
            .call(())
            .expect("hour");
        assert_eq!(hora, 21.25);
        let noite: bool = module
            .get::<Function>("is_night")
            .expect("M.is_night")
            .call(())
            .expect("is_night");
        assert!(noite);
        let chuva: f64 = module
            .get::<Function>("rain")
            .expect("M.rain")
            .call(())
            .expect("rain");
        assert!((chuva - 0.2).abs() < 1e-6, "rain = {chuva}");
        let chove: bool = module
            .get::<Function>("raining")
            .expect("M.raining")
            .call(())
            .expect("raining");
        assert!(!chove);
        assert_eq!(module.get::<f64>("NIGHT_FROM").expect("NIGHT_FROM"), 20.0);
        assert_eq!(module.get::<f64>("NIGHT_TO").expect("NIGHT_TO"), 5.0);
        assert_eq!(module.get::<f64>("RAIN_HEAVY").expect("RAIN_HEAVY"), 0.3);

        // As closures leem o snapshot vivo, não um valor capturado.
        clock.set(snap(None, 0.5));
        let hora: Option<f64> = module
            .get::<Function>("hour")
            .expect("M.hour")
            .call(())
            .expect("hour sem relógio");
        assert_eq!(hora, None);
    }

    #[test]
    fn bridge_nao_substitui_funcoes_existentes() {
        let lua = Lua::new();
        lua.globals()
            .set("viber", lua.create_table().expect("tabela viber"))
            .expect("global viber");
        lua.load("viber.vael = { vault_add = function() return 7 end }")
            .exec()
            .expect("vael antigo");
        install(&lua, &VaelClock::default()).expect("instalar");
        let valor: i64 = lua
            .load("return viber.vael.vault_add()")
            .eval()
            .expect("vault_add intacto");
        assert_eq!(valor, 7);
        assert!(
            lua.load("return type(viber.vael.clock_hour) == 'function'")
                .eval::<bool>()
                .expect("clock_hour")
        );
    }

    #[test]
    fn sistema_escreve_o_snapshot() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(VaelClockPlugin);
        app.insert_resource(DayCycleState::from_parts(
            21.0 * 60.0 + 30.0,
            0.0,
            6.0 * 60.0,
            20.0 * 60.0,
            1.0,
            0.1,
            false,
            45.0,
            90.0,
            -5.0,
            1.0,
            1.0,
            0.0,
        ));
        app.insert_resource(WeatherState {
            wind: [0.0, 0.0],
            wind_strength: 0.0,
            clouds: 0.0,
            rain: 0.4,
            cycle: false,
        });

        app.update();

        let clock = app.world().resource::<VaelClock>().get();
        assert_eq!(clock.hour, Some(21.5));
        assert!((clock.rain - 0.4).abs() < 1e-6, "rain = {}", clock.rain);
    }

    #[test]
    fn sistema_sem_recursos_limpa_o_snapshot() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(VaelClockPlugin);
        app.world()
            .resource::<VaelClock>()
            .set(snap(Some(22.0), 1.0));

        app.update();

        let clock = app.world().resource::<VaelClock>().get();
        assert_eq!(clock, snap(None, 0.0));
    }
}
