//! Estado e capacidades do ar-condicionado. Porte de `msmart/device/AC/device.py`.

use std::net::SocketAddr;

use crate::ac::{self, RawCapabilities, Response, SetState};
use crate::lan::{Error, Lan, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Auto = 1,
    Cool = 2,
    Dry = 3,
    Heat = 4,
    FanOnly = 5,
    SmartDry = 6,
}

impl Mode {
    pub const ALL: [Mode; 5] = [Mode::Auto, Mode::Cool, Mode::Dry, Mode::Heat, Mode::FanOnly];

    fn from_u8(v: u8) -> Mode {
        match v {
            1 => Mode::Auto,
            2 => Mode::Cool,
            3 => Mode::Dry,
            4 => Mode::Heat,
            6 => Mode::SmartDry,
            _ => Mode::FanOnly,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Swing {
    Off = 0x0,
    Vertical = 0xC,
    Horizontal = 0x3,
    Both = 0xF,
}

impl Swing {
    pub const ALL: [Swing; 4] = [Swing::Off, Swing::Vertical, Swing::Horizontal, Swing::Both];

    fn from_u8(v: u8) -> Swing {
        match v {
            0xC => Swing::Vertical,
            0x3 => Swing::Horizontal,
            0xF => Swing::Both,
            _ => Swing::Off,
        }
    }
}

/// Velocidades do ventilador (0–100 é percentual; acima de 100 é automática).
pub mod fan {
    pub const AUTO: u8 = 102;
    pub const MAX: u8 = 100;
    pub const HIGH: u8 = 80;
    pub const MEDIUM: u8 = 60;
    pub const LOW: u8 = 40;
    pub const SILENT: u8 = 20;
    pub const ALL: [u8; 6] = [AUTO, SILENT, LOW, MEDIUM, HIGH, MAX];

    /// Velocidades acima de 100 são variações de "automática".
    pub fn matches(current: u8, speed: u8) -> bool {
        (current > 100 && speed == AUTO) || current == speed
    }
}

/// Posições das aletas (0 = padrão do aparelho).
pub const SWING_ANGLES: [u8; 6] = [0, 1, 25, 50, 75, 100];

fn angle_from_u8(v: u8) -> u8 {
    if SWING_ANGLES.contains(&v) {
        v
    } else {
        0
    }
}

const BREEZE_OFF: u8 = 1;
const BREEZE_AWAY: u8 = 2;
const BREEZELESS: u8 = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub power: bool,
    pub target_temperature: f32,
    pub mode: Mode,
    pub fan_speed: u8,
    pub swing: Swing,
    pub eco: bool,
    pub turbo: bool,
    pub sleep: bool,
    pub purifier: bool,
    pub freeze_protection: bool,
    pub follow_me: bool,
    pub fahrenheit: bool,
    pub target_humidity: u8,
    pub aux_heat: bool,
    pub independent_aux_heat: bool,
    pub sound: bool,
    pub vertical_swing_angle: u8,
    pub horizontal_swing_angle: u8,
    breeze_mode: u8,
    pub indoor_temperature: Option<f32>,
    pub outdoor_temperature: Option<f32>,
    pub self_clean_active: bool,
    pub display_on: bool,
    pub filter_alert: bool,
    pub error_code: u8,
}

impl Default for State {
    fn default() -> State {
        State {
            power: false,
            target_temperature: 17.0,
            mode: Mode::Auto,
            fan_speed: fan::AUTO,
            swing: Swing::Off,
            eco: false,
            turbo: false,
            sleep: false,
            purifier: false,
            freeze_protection: false,
            follow_me: false,
            fahrenheit: false,
            target_humidity: 40,
            aux_heat: false,
            independent_aux_heat: false,
            sound: false,
            vertical_swing_angle: 0,
            horizontal_swing_angle: 0,
            breeze_mode: BREEZE_OFF,
            indoor_temperature: None,
            outdoor_temperature: None,
            self_clean_active: false,
            display_on: false,
            filter_alert: false,
            error_code: 0,
        }
    }
}

impl State {
    pub fn breeze_away(&self) -> bool {
        self.breeze_mode == BREEZE_AWAY
    }

    pub fn set_breeze_away(&mut self, on: bool) {
        self.breeze_mode = if on { BREEZE_AWAY } else { BREEZE_OFF };
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Capabilities {
    pub modes: Vec<Mode>,
    pub swing_modes: Vec<Swing>,
    pub fan_speeds: Vec<u8>,
    pub custom_fan_speed: bool,
    pub eco: bool,
    pub turbo: bool,
    pub freeze_protection: bool,
    pub purifier: bool,
    pub display_control: bool,
    pub humidity: bool,
    pub vertical_swing_angle: bool,
    pub horizontal_swing_angle: bool,
    pub breeze_away: bool,
    pub breeze_control: bool,
    pub breezeless: bool,
    pub self_clean: bool,
    pub min_temperature: f32,
    pub max_temperature: f32,
}

impl Default for Capabilities {
    /// O que a msmart assume antes de consultar o aparelho.
    fn default() -> Capabilities {
        Capabilities {
            modes: Mode::ALL.to_vec(),
            swing_modes: Swing::ALL.to_vec(),
            fan_speeds: fan::ALL.to_vec(),
            custom_fan_speed: true,
            eco: true,
            turbo: true,
            freeze_protection: true,
            purifier: true,
            display_control: true,
            humidity: false,
            vertical_swing_angle: false,
            horizontal_swing_angle: false,
            breeze_away: false,
            breeze_control: false,
            breezeless: false,
            self_clean: false,
            min_temperature: 16.0,
            max_temperature: 30.0,
        }
    }
}

impl Capabilities {
    fn from_raw(raw: &RawCapabilities) -> Capabilities {
        let mut modes = vec![Mode::FanOnly];
        for (name, mode) in [("dry_mode", Mode::Dry), ("cool_mode", Mode::Cool), ("heat_mode", Mode::Heat), ("auto_mode", Mode::Auto)] {
            if raw.get(name) {
                modes.push(mode);
            }
        }
        if raw.get("humidity_manual_set") {
            modes.push(Mode::SmartDry);
        }
        let mut swing_modes = vec![Swing::Off];
        let (h, v) = (raw.get("swing_horizontal"), raw.get("swing_vertical"));
        if h {
            swing_modes.push(Swing::Horizontal);
        }
        if v {
            swing_modes.push(Swing::Vertical);
        }
        if h && v {
            swing_modes.push(Swing::Both);
        }
        let mut fan_speeds = Vec::new();
        for (name, speed) in [("silent", fan::SILENT), ("low", fan::LOW), ("medium", fan::MEDIUM), ("high", fan::HIGH), ("auto", fan::AUTO)] {
            if raw.fan(name) {
                fan_speeds.push(speed);
            }
        }
        let custom = raw.get("fan_custom");
        if custom {
            fan_speeds.push(fan::MAX);
        }
        let breeze_control = raw.get("breeze_control");
        Capabilities {
            modes,
            swing_modes,
            fan_speeds,
            custom_fan_speed: custom,
            eco: raw.get("eco"),
            turbo: raw.get("turbo_heat") || raw.get("turbo_cool"),
            freeze_protection: raw.get("freeze_protection"),
            purifier: raw.get("anion"),
            display_control: raw.get("display_control"),
            humidity: raw.get("humidity_auto_set") || raw.get("humidity_manual_set"),
            vertical_swing_angle: raw.get("swing_vertical_angle"),
            horizontal_swing_angle: raw.get("swing_horizontal_angle"),
            breeze_away: breeze_control || raw.get("breeze_away"),
            breeze_control,
            breezeless: breeze_control || raw.get("breezeless"),
            self_clean: raw.get("self_clean"),
            min_temperature: raw.min_temperature(),
            max_temperature: raw.max_temperature(),
        }
    }

    /// Propriedades consultadas a cada atualização.
    fn properties(&self) -> Vec<u16> {
        let mut props = Vec::new();
        if self.breeze_control {
            props.push(ac::PROP_BREEZE_CONTROL);
        } else {
            if self.breeze_away {
                props.push(ac::PROP_BREEZE_AWAY);
            }
            if self.breezeless {
                props.push(ac::PROP_BREEZELESS);
            }
        }
        if self.self_clean {
            props.push(ac::PROP_SELF_CLEAN);
        }
        if self.horizontal_swing_angle {
            props.push(ac::PROP_SWING_LR_ANGLE);
        }
        if self.vertical_swing_angle {
            props.push(ac::PROP_SWING_UD_ANGLE);
        }
        props
    }
}

pub struct Device {
    pub id: u64,
    lan: Lan,
    pub state: State,
    pub caps: Capabilities,
}

impl Device {
    pub fn new(addr: SocketAddr, id: u64, token: &str, key: &str) -> Result<Device> {
        Ok(Device { id, lan: Lan::new(addr, id, token, key)?, state: State::default(), caps: Capabilities::default() })
    }

    pub fn addr(&self) -> SocketAddr {
        self.lan.addr()
    }

    pub fn set_addr(&mut self, addr: SocketAddr) {
        self.lan.set_addr(addr);
    }

    pub fn authenticate(&mut self) -> Result<()> {
        self.lan.authenticate()
    }

    pub fn disconnect(&mut self) {
        self.lan.disconnect();
    }

    /// Envia os frames e devolve as respostas válidas. Erro se o aparelho não respondeu.
    fn send(&mut self, frames: &[Vec<u8>]) -> Result<Vec<Response>> {
        let mut raw = Vec::new();
        for f in frames {
            match self.lan.send(f) {
                Ok(r) => raw.extend(r),
                // Igual à msmart: um timeout num dos comandos não descarta o resto
                Err(Error::Timeout(e)) => log::warn!("Sem resposta de {}: {e}", self.addr()),
                Err(e) => return Err(e),
            }
        }
        if raw.is_empty() {
            return Err(Error::Timeout("sem resposta do aparelho".into()));
        }
        Ok(raw
            .iter()
            .filter_map(|f| Response::parse(f).map_err(|e| log::error!("{e}")).ok())
            .collect())
    }

    pub fn get_capabilities(&mut self) -> Result<()> {
        let mut raw = self.send(&[ac::get_capabilities(false)])?.into_iter().find_map(|r| match r {
            Response::Capabilities(c) => Some(c),
            _ => None,
        });
        let Some(caps) = raw.as_mut() else {
            log::error!("O aparelho não informou as capacidades");
            return Ok(());
        };
        if caps.additional {
            let more = self.send(&[ac::get_capabilities(true)])?.into_iter().find_map(|r| match r {
                Response::Capabilities(c) => Some(c),
                _ => None,
            });
            match more {
                Some(m) => caps.merge(m),
                None => log::warn!("O aparelho não informou as capacidades adicionais"),
            }
        }
        log::debug!("Capacidades: {caps:?}");
        self.caps = Capabilities::from_raw(caps);
        Ok(())
    }

    pub fn refresh(&mut self) -> Result<()> {
        let mut frames = vec![ac::get_state()];
        let props = self.caps.properties();
        if !props.is_empty() {
            frames.push(ac::get_properties(&props));
        }
        for r in self.send(&frames)? {
            self.update(r);
        }
        Ok(())
    }

    /// Envia o estado local; propriedades que mudaram desde `before` vão num comando à parte.
    pub fn apply(&mut self, before: &State) -> Result<()> {
        let s = &self.state;
        let cmd = SetState {
            beep: s.sound,
            power: s.power,
            target_temperature: s.target_temperature,
            mode: s.mode as u8,
            fan_speed: s.fan_speed,
            swing_mode: s.swing as u8,
            eco: s.eco,
            turbo: s.turbo,
            fahrenheit: s.fahrenheit,
            sleep: s.sleep,
            freeze_protection: s.freeze_protection,
            follow_me: s.follow_me,
            purifier: s.purifier,
            target_humidity: s.target_humidity,
            aux_heat: s.aux_heat,
            independent_aux_heat: s.independent_aux_heat,
        };
        let mut props = Vec::new();
        if s.horizontal_swing_angle != before.horizontal_swing_angle {
            props.push((ac::PROP_SWING_LR_ANGLE, vec![s.horizontal_swing_angle]));
        }
        if s.vertical_swing_angle != before.vertical_swing_angle {
            props.push((ac::PROP_SWING_UD_ANGLE, vec![s.vertical_swing_angle]));
        }
        if s.breeze_mode != before.breeze_mode {
            if self.caps.breeze_control {
                props.push((ac::PROP_BREEZE_CONTROL, vec![s.breeze_mode]));
            } else if s.breeze_mode == BREEZELESS || before.breeze_mode == BREEZELESS {
                props.push((ac::PROP_BREEZELESS, vec![u8::from(s.breeze_mode == BREEZELESS)]));
            } else {
                props.push((ac::PROP_BREEZE_AWAY, vec![if s.breeze_away() { 2 } else { 1 }]));
            }
        }
        for r in self.send(&[cmd.encode()])? {
            self.update(r);
        }
        if !props.is_empty() {
            self.apply_properties(props)?;
        }
        Ok(())
    }

    pub fn start_self_clean(&mut self) -> Result<()> {
        self.apply_properties(vec![(ac::PROP_SELF_CLEAN, vec![1])])
    }

    fn apply_properties(&mut self, mut props: Vec<(u16, Vec<u8>)>) -> Result<()> {
        props.push((ac::PROP_BUZZER, vec![u8::from(self.state.sound)]));
        for r in self.send(&[ac::set_properties(&props)])? {
            self.update(r);
        }
        Ok(())
    }

    fn update(&mut self, r: Response) {
        let s = &mut self.state;
        match r {
            Response::State(r) => {
                s.power = r.power;
                s.target_temperature = r.target_temperature;
                s.mode = Mode::from_u8(r.mode);
                s.fan_speed = if self.caps.custom_fan_speed || fan::ALL.contains(&r.fan_speed) { r.fan_speed } else { fan::AUTO };
                s.swing = Swing::from_u8(r.swing_mode);
                s.eco = r.eco;
                s.turbo = r.turbo;
                s.freeze_protection = r.freeze_protection.unwrap_or(false);
                s.sleep = r.sleep;
                s.indoor_temperature = r.indoor_temperature;
                s.outdoor_temperature = r.outdoor_temperature;
                s.display_on = r.display_on;
                s.fahrenheit = r.fahrenheit;
                s.filter_alert = r.filter_alert;
                s.follow_me = r.follow_me;
                s.purifier = r.purifier;
                if let Some(h) = r.target_humidity {
                    s.target_humidity = h;
                }
                s.aux_heat = r.aux_heat;
                s.independent_aux_heat = r.independent_aux_heat;
                s.error_code = r.error_code;
            }
            Response::Properties(p) => {
                let p = &p.0;
                if let Some(&v) = p.get(&ac::PROP_SWING_LR_ANGLE) {
                    s.horizontal_swing_angle = angle_from_u8(v);
                }
                if let Some(&v) = p.get(&ac::PROP_SWING_UD_ANGLE) {
                    s.vertical_swing_angle = angle_from_u8(v);
                }
                if let Some(&v) = p.get(&ac::PROP_SELF_CLEAN) {
                    s.self_clean_active = v != 0;
                }
                if let Some(&v) = p.get(&ac::PROP_BREEZE_CONTROL) {
                    s.breeze_mode = if (1..=4).contains(&v) { v } else { BREEZE_OFF };
                } else {
                    if let Some(&v) = p.get(&ac::PROP_BREEZE_AWAY) {
                        s.breeze_mode = if v != 0 { BREEZE_AWAY } else { BREEZE_OFF };
                    }
                    if let Some(&v) = p.get(&ac::PROP_BREEZELESS) {
                        s.breeze_mode = if v != 0 { BREEZELESS } else { BREEZE_OFF };
                    }
                }
            }
            Response::Capabilities(_) | Response::Other(_) => log::debug!("Resposta ignorada: {r:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_of_this_unit() {
        let raw1 = Response::parse(&hex::decode("aa31ac00000000000203b507120201011402010015020101160201001a02010110020101250207203c203c203c000100bfcf").unwrap()).unwrap();
        let raw2 = Response::parse(&hex::decode("aa2fac00000000000203b5081e0201011302010022020100190201003900010142000101090001010a00010100006de7").unwrap()).unwrap();
        let (Response::Capabilities(mut a), Response::Capabilities(b)) = (raw1, raw2) else { panic!() };
        a.merge(b);
        let c = Capabilities::from_raw(&a);
        // Mesmo resultado que a msmart deu para este aparelho
        assert_eq!(c.modes, vec![Mode::FanOnly, Mode::Dry, Mode::Cool, Mode::Auto]);
        assert_eq!(c.swing_modes, vec![Swing::Off, Swing::Horizontal, Swing::Vertical, Swing::Both]);
        assert_eq!(c.fan_speeds, vec![fan::SILENT, fan::LOW, fan::MEDIUM, fan::HIGH, fan::AUTO, fan::MAX]);
        assert!(c.custom_fan_speed && c.eco && c.turbo && c.purifier && c.self_clean && c.breeze_away);
        assert!(c.vertical_swing_angle && c.horizontal_swing_angle);
        assert!(!c.freeze_protection && !c.display_control && !c.breeze_control && !c.breezeless && !c.humidity);
        let mut props = c.properties();
        props.sort();
        assert_eq!(props, vec![0x09, 0x0A, 0x39, 0x42]);
    }
}
