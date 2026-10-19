//! Frames de comando e resposta do ar-condicionado (tipo 0xAC).
//!
//! Porte de `msmart/frame.py`, `msmart/crc8.py` e `msmart/device/AC/command.py`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU8, Ordering};

const DEVICE_TYPE: u8 = 0xAC;
const FRAME_CONTROL: u8 = 0x02;
const FRAME_QUERY: u8 = 0x03;
const CONTROL_SOURCE: u8 = 0x02; // controle pelo app

pub const PROP_SWING_UD_ANGLE: u16 = 0x0009;
pub const PROP_SWING_LR_ANGLE: u16 = 0x000A;
pub const PROP_BREEZELESS: u16 = 0x0018;
pub const PROP_BUZZER: u16 = 0x001A;
pub const PROP_SELF_CLEAN: u16 = 0x0039;
pub const PROP_BREEZE_AWAY: u16 = 0x0042;
pub const PROP_BREEZE_CONTROL: u16 = 0x0043;

#[rustfmt::skip]
const CRC8_854: [u8; 256] = [
    0x00, 0x5E, 0xBC, 0xE2, 0x61, 0x3F, 0xDD, 0x83, 0xC2, 0x9C, 0x7E, 0x20, 0xA3, 0xFD, 0x1F, 0x41,
    0x9D, 0xC3, 0x21, 0x7F, 0xFC, 0xA2, 0x40, 0x1E, 0x5F, 0x01, 0xE3, 0xBD, 0x3E, 0x60, 0x82, 0xDC,
    0x23, 0x7D, 0x9F, 0xC1, 0x42, 0x1C, 0xFE, 0xA0, 0xE1, 0xBF, 0x5D, 0x03, 0x80, 0xDE, 0x3C, 0x62,
    0xBE, 0xE0, 0x02, 0x5C, 0xDF, 0x81, 0x63, 0x3D, 0x7C, 0x22, 0xC0, 0x9E, 0x1D, 0x43, 0xA1, 0xFF,
    0x46, 0x18, 0xFA, 0xA4, 0x27, 0x79, 0x9B, 0xC5, 0x84, 0xDA, 0x38, 0x66, 0xE5, 0xBB, 0x59, 0x07,
    0xDB, 0x85, 0x67, 0x39, 0xBA, 0xE4, 0x06, 0x58, 0x19, 0x47, 0xA5, 0xFB, 0x78, 0x26, 0xC4, 0x9A,
    0x65, 0x3B, 0xD9, 0x87, 0x04, 0x5A, 0xB8, 0xE6, 0xA7, 0xF9, 0x1B, 0x45, 0xC6, 0x98, 0x7A, 0x24,
    0xF8, 0xA6, 0x44, 0x1A, 0x99, 0xC7, 0x25, 0x7B, 0x3A, 0x64, 0x86, 0xD8, 0x5B, 0x05, 0xE7, 0xB9,
    0x8C, 0xD2, 0x30, 0x6E, 0xED, 0xB3, 0x51, 0x0F, 0x4E, 0x10, 0xF2, 0xAC, 0x2F, 0x71, 0x93, 0xCD,
    0x11, 0x4F, 0xAD, 0xF3, 0x70, 0x2E, 0xCC, 0x92, 0xD3, 0x8D, 0x6F, 0x31, 0xB2, 0xEC, 0x0E, 0x50,
    0xAF, 0xF1, 0x13, 0x4D, 0xCE, 0x90, 0x72, 0x2C, 0x6D, 0x33, 0xD1, 0x8F, 0x0C, 0x52, 0xB0, 0xEE,
    0x32, 0x6C, 0x8E, 0xD0, 0x53, 0x0D, 0xEF, 0xB1, 0xF0, 0xAE, 0x4C, 0x12, 0x91, 0xCF, 0x2D, 0x73,
    0xCA, 0x94, 0x76, 0x28, 0xAB, 0xF5, 0x17, 0x49, 0x08, 0x56, 0xB4, 0xEA, 0x69, 0x37, 0xD5, 0x8B,
    0x57, 0x09, 0xEB, 0xB5, 0x36, 0x68, 0x8A, 0xD4, 0x95, 0xCB, 0x29, 0x77, 0xF4, 0xAA, 0x48, 0x16,
    0xE9, 0xB7, 0x55, 0x0B, 0x88, 0xD6, 0x34, 0x6A, 0x2B, 0x75, 0x97, 0xC9, 0x4A, 0x14, 0xF6, 0xA8,
    0x74, 0x2A, 0xC8, 0x96, 0x15, 0x4B, 0xA9, 0xF7, 0xB6, 0xE8, 0x0A, 0x54, 0xD7, 0x89, 0x6B, 0x35,
];

fn crc8(data: &[u8]) -> u8 {
    data.iter().fold(0, |crc, &b| CRC8_854[(crc ^ b) as usize])
}

fn checksum(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |s, &b| s.wrapping_add(b)).wrapping_neg()
}

static MESSAGE_ID: AtomicU8 = AtomicU8::new(0);

/// Monta um frame AA..: cabeçalho, dados + ID da mensagem + CRC8, checksum.
fn frame(frame_type: u8, data: &[u8]) -> Vec<u8> {
    let mut payload = data.to_vec();
    payload.push(MESSAGE_ID.fetch_add(1, Ordering::Relaxed).wrapping_add(1));
    payload.push(crc8(&payload));
    let mut f = vec![0xAA, (payload.len() + 10) as u8, DEVICE_TYPE, 0, 0, 0, 0, 0, 0, frame_type];
    f.extend_from_slice(&payload);
    f.push(checksum(&f[1..]));
    f
}

// ---------- comandos ----------

pub fn get_capabilities(additional: bool) -> Vec<u8> {
    if additional {
        frame(FRAME_QUERY, &[0xB5, 0x01, 0x01, 0x01])
    } else {
        frame(FRAME_QUERY, &[0xB5, 0x01, 0x00])
    }
}

pub fn get_state() -> Vec<u8> {
    let mut d = [0u8; 21];
    d[..7].copy_from_slice(&[0x41, 0x81, 0x00, 0xFF, 0x03, 0xFF, 0x00]);
    d[7] = 0x02; // temperatura interna
    d[20] = 0x03;
    frame(FRAME_QUERY, &d)
}

pub fn get_properties(props: &[u16]) -> Vec<u8> {
    let mut d = vec![0xB1, props.len() as u8];
    for p in props {
        d.extend_from_slice(&p.to_le_bytes());
    }
    frame(FRAME_QUERY, &d)
}

pub fn set_properties(props: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut d = vec![0xB0, props.len() as u8];
    for (id, value) in props {
        d.extend_from_slice(&id.to_le_bytes());
        d.push(value.len() as u8);
        d.extend_from_slice(value);
    }
    frame(FRAME_CONTROL, &d)
}

#[derive(Clone, Debug, Default)]
pub struct SetState {
    pub beep: bool,
    pub power: bool,
    pub target_temperature: f32,
    pub mode: u8,
    pub fan_speed: u8,
    pub swing_mode: u8,
    pub eco: bool,
    pub turbo: bool,
    pub fahrenheit: bool,
    pub sleep: bool,
    pub freeze_protection: bool,
    pub follow_me: bool,
    pub purifier: bool,
    pub target_humidity: u8,
    pub aux_heat: bool,
    pub independent_aux_heat: bool,
}

impl SetState {
    pub fn encode(&self) -> Vec<u8> {
        let integral = self.target_temperature.trunc() as i32;
        let half = self.target_temperature.fract() > 0.0;
        let (mut temp, temp_alt) = if (17..=30).contains(&integral) {
            (((integral - 16) & 0xF) as u8, 0)
        } else {
            (0, ((integral - 12) & 0x1F) as u8)
        };
        if half {
            temp |= 0x10;
        }
        let b = |cond: bool, bit: u8| if cond { bit } else { 0 };
        frame(
            FRAME_CONTROL,
            &[
                0x40,
                CONTROL_SOURCE | b(self.beep, 0x40) | b(self.power, 0x01),
                temp | (self.mode & 0x7) << 5,
                self.fan_speed,
                0x7F,
                0x7F,
                0x00,
                0x30 | (self.swing_mode & 0x3F),
                b(self.follow_me, 0x80) | b(self.turbo, 0x20),
                b(self.eco, 0x80) | b(self.purifier, 0x20) | b(self.aux_heat, 0x08),
                b(self.sleep, 0x01) | b(self.turbo, 0x02) | b(self.fahrenheit, 0x04),
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                temp_alt,
                self.target_humidity & 0x7F,
                0x00,
                b(self.freeze_protection, 0x80),
                b(self.independent_aux_heat, 0x08),
                0x00,
            ],
        )
    }
}

// ---------- respostas ----------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StateResponse {
    pub power: bool,
    pub target_temperature: f32,
    pub mode: u8,
    pub fan_speed: u8,
    pub swing_mode: u8,
    pub turbo: bool,
    pub eco: bool,
    pub sleep: bool,
    pub fahrenheit: bool,
    pub indoor_temperature: Option<f32>,
    pub outdoor_temperature: Option<f32>,
    pub filter_alert: bool,
    pub display_on: bool,
    pub follow_me: bool,
    pub purifier: bool,
    pub aux_heat: bool,
    pub independent_aux_heat: bool,
    pub error_code: u8,
    pub target_humidity: Option<u8>,
    pub freeze_protection: Option<bool>,
}

fn parse_temperature(data: u8, decimals: f32, fahrenheit: bool) -> Option<f32> {
    if data == 0xFF {
        return None;
    }
    let t = (data as f32 - 50.0) / 2.0;
    let sign = if t >= 0.0 { 1.0 } else { -1.0 };
    if !fahrenheit && decimals != 0.0 {
        return Some(t.trunc() + sign * decimals);
    }
    if decimals >= 0.5 {
        return Some(t.trunc() + sign * 0.5);
    }
    Some(t)
}

impl StateResponse {
    fn parse(p: &[u8]) -> Option<StateResponse> {
        if p.len() < 17 {
            return None;
        }
        let fahrenheit = p[10] & 0x04 != 0;
        let mut target = (p[2] & 0xF) as f32 + 16.0 + if p[2] & 0x10 != 0 { 0.5 } else { 0.0 };
        let alt = p[13] & 0x1F;
        if alt != 0 {
            target = alt as f32 + 12.0 + if p[2] & 0x10 != 0 { 0.5 } else { 0.0 };
        }
        Some(StateResponse {
            power: p[1] & 0x01 != 0,
            target_temperature: target,
            mode: (p[2] >> 5) & 0x7,
            fan_speed: p[3] & 0x7F,
            swing_mode: p[7] & 0xF,
            turbo: p[8] & 0x20 != 0 || p[10] & 0x02 != 0,
            independent_aux_heat: p[8] & 0x40 != 0,
            follow_me: p[8] & 0x80 != 0,
            eco: p[9] & 0x10 != 0,
            purifier: p[9] & 0x20 != 0,
            aux_heat: p[9] & 0x08 != 0,
            sleep: p[10] & 0x01 != 0,
            fahrenheit,
            indoor_temperature: parse_temperature(p[11], (p[15] & 0xF) as f32 / 10.0, fahrenheit),
            outdoor_temperature: parse_temperature(p[12], (p[15] >> 4) as f32 / 10.0, fahrenheit),
            filter_alert: p[13] & 0x20 != 0,
            display_on: p[14] != 0x70,
            error_code: p[16],
            target_humidity: p.get(19).map(|h| h & 0x7F),
            freeze_protection: p.get(21).map(|f| f & 0x80 != 0),
        })
    }
}

/// Valores das propriedades já decodificados.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Properties(pub BTreeMap<u16, u8>);

impl Properties {
    fn parse(p: &[u8]) -> Properties {
        let mut out = BTreeMap::new();
        let count = p.get(1).copied().unwrap_or(0);
        let mut rest = p.get(2..).unwrap_or_default();
        for _ in 0..count {
            if rest.len() < 4 {
                break;
            }
            let id = u16::from_le_bytes([rest[0], rest[1]]);
            let size = rest[3] as usize;
            let data = rest.get(4..4 + size).unwrap_or_default();
            if rest[2] & 0x10 != 0 {
                log::error!("Propriedade 0x{id:04X} falhou, resultado 0x{:02X}", rest[2]);
            }
            if let Some(&v) = data.first() {
                let value = match id {
                    PROP_BREEZE_AWAY => u8::from(v == 2),
                    PROP_BREEZELESS | PROP_SELF_CLEAN => u8::from(v != 0),
                    PROP_BUZZER => {
                        rest = rest.get(4 + size..).unwrap_or_default();
                        continue;
                    }
                    _ => v,
                };
                out.insert(id, value);
            }
            rest = rest.get(4 + size..).unwrap_or_default();
        }
        Properties(out)
    }
}

/// Capacidades brutas lidas da resposta B5 (nome → valor).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawCapabilities {
    pub flags: BTreeMap<&'static str, bool>,
    pub temps: BTreeMap<&'static str, f32>,
    pub additional: bool,
}

impl RawCapabilities {
    fn parse(p: &[u8]) -> RawCapabilities {
        let mut caps = RawCapabilities::default();
        let count = p.get(1).copied().unwrap_or(0);
        let mut rest = p.get(2..).unwrap_or_default();
        for _ in 0..count {
            if rest.len() < 3 {
                break;
            }
            let size = rest[2] as usize;
            let id = u16::from_le_bytes([rest[0], rest[1]]);
            let value = rest.get(3..3 + size).unwrap_or_default();
            if let Some(&v) = value.first() {
                caps.read(id, v, value);
            }
            rest = rest.get(3 + size..).unwrap_or_default();
        }
        if rest.len() > 1 {
            caps.additional = rest[rest.len() - 2] != 0;
        }
        caps
    }

    fn read(&mut self, id: u16, v: u8, value: &[u8]) {
        let mut set = |name: &'static str, on: bool| {
            self.flags.insert(name, on);
        };
        let any = |list: &[u8]| list.contains(&v);
        match id {
            0x021E => set("anion", v == 1),
            0x0219 => set("aux_electric_heat", v == 1),
            0x0042 => set("breeze_away", v == 1),
            0x0043 => set("breeze_control", v == 1),
            0x0018 => set("breezeless", v == 1),
            0x022C => set("sound", v == 1),
            0x0224 => set("display_control", any(&[1, 2, 100])),
            0x0216 => set("energy_stats", any(&[2, 3, 4, 5])),
            0x0222 => set("fahrenheit", v == 0),
            0x0210 => {
                set("fan_silent", any(&[6, 9]));
                set("fan_low", any(&[3, 4, 5, 6, 7, 9]));
                set("fan_medium", any(&[5, 6, 7]));
                set("fan_high", any(&[3, 4, 5, 6, 7, 9]));
                set("fan_auto", any(&[4, 5, 6, 9]));
                set("fan_custom", v == 1);
            }
            0x0217 => set("filter_notice", any(&[1, 2, 4])),
            0x021F => {
                set("humidity_auto_set", any(&[1, 2]));
                set("humidity_manual_set", any(&[2, 3]));
            }
            0x0214 => {
                set("heat_mode", any(&[1, 2, 4, 6, 7, 9, 10, 11, 12, 13]));
                set("cool_mode", any(&[0, 1, 3, 4, 5, 6, 7, 8, 9, 11, 13, 14, 15]));
                set("dry_mode", any(&[0, 1, 5, 6, 9, 11, 13, 14, 15]));
                set("auto_mode", any(&[0, 1, 2, 7, 8, 9, 13, 14]));
                set("aux_heat_mode", v == 9);
                set("aux_mode", any(&[9, 10, 11, 13, 14, 15]));
            }
            0x0212 => set("eco", any(&[1, 2])),
            0x0213 => set("freeze_protection", v == 1),
            0x021A => {
                set("turbo_heat", any(&[1, 3]));
                set("turbo_cool", any(&[0, 1]));
            }
            0x0039 => set("self_clean", v == 1),
            0x000A => set("swing_horizontal_angle", v == 1),
            0x0009 => set("swing_vertical_angle", v == 1),
            0x0215 => {
                set("swing_horizontal", any(&[1, 3]));
                set("swing_vertical", any(&[0, 1]));
            }
            0x0225 if value.len() >= 6 => {
                for (i, name) in [
                    "cool_min", "cool_max", "auto_min", "auto_max", "heat_min", "heat_max",
                ]
                .into_iter()
                .enumerate()
                {
                    self.temps.insert(name, value[i] as f32 * 0.5);
                }
            }
            _ => log::debug!("Capacidade ignorada 0x{id:04X}: {}", hex::encode(value)),
        }
    }

    pub fn merge(&mut self, other: RawCapabilities) {
        self.flags.extend(other.flags);
        self.temps.extend(other.temps);
    }

    pub fn get(&self, name: &str) -> bool {
        self.flags.get(name).copied().unwrap_or(false)
    }

    /// Mesma regra da msmart: se veio alguma capacidade de ventilador, usa-as
    /// (velocidade livre libera todas); senão assume baixa/média/alta/auto.
    pub fn fan(&self, speed: &str) -> bool {
        if self.flags.keys().any(|k| k.starts_with("fan_")) {
            self.get(&format!("fan_{speed}")) || self.get("fan_custom")
        } else {
            matches!(speed, "low" | "medium" | "high" | "auto")
        }
    }

    pub fn min_temperature(&self) -> f32 {
        ["cool_min", "auto_min", "heat_min"]
            .iter()
            .map(|k| self.temps.get(k).copied().unwrap_or(16.0))
            .fold(f32::INFINITY, f32::min)
    }

    pub fn max_temperature(&self) -> f32 {
        ["cool_max", "auto_max", "heat_max"]
            .iter()
            .map(|k| self.temps.get(k).copied().unwrap_or(30.0))
            .fold(f32::NEG_INFINITY, f32::max)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    State(StateResponse),
    Capabilities(RawCapabilities),
    Properties(Properties),
    Other(u8),
}

impl Response {
    pub fn parse(frame: &[u8]) -> Result<Response, String> {
        if frame.len() < 13 {
            return Err(format!("frame curto demais: {}", hex::encode(frame)));
        }
        let expected = checksum(&frame[1..frame.len() - 1]);
        if expected != frame[frame.len() - 1] {
            return Err(format!("frame com checksum inválido: {}", hex::encode(frame)));
        }
        if frame[2] != DEVICE_TYPE {
            return Err(format!("tipo de aparelho inesperado: 0x{:X}", frame[2]));
        }
        let frame_type = frame[9];
        let id = frame[10];
        let is_props = matches!(id, 0xB0 | 0xB1);
        if !is_props {
            // Alguns aparelhos usam CRC8, outros um segundo checksum
            let p = &frame[10..frame.len() - 1];
            let (body, last) = p.split_at(p.len() - 1);
            if crc8(body) != last[0] && checksum(body) != last[0] {
                return Err(format!("frame com CRC inválido: {}", hex::encode(frame)));
            }
        }
        let body = &frame[10..frame.len() - 2];
        Ok(match id {
            0xC0 => StateResponse::parse(body).map(Response::State).ok_or("estado curto demais")?,
            0xB5 if frame_type == FRAME_QUERY => Response::Capabilities(RawCapabilities::parse(body)),
            0xB0 | 0xB1 => Response::Properties(Properties::parse(body)),
            other => Response::Other(other),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Vec<u8> {
        hex::decode(s).unwrap()
    }

    #[test]
    fn frames_match_msmart() {
        // A msmart numera as mensagens a partir de 1; os testes rodam em paralelo,
        // então compara só o que não depende do ID.
        let f = get_capabilities(false);
        assert_eq!(f[..13], h("aa0fac00000000000003b50100")[..]);
        let f = get_state();
        assert_eq!(f[..31], h("aa21ac00000000000003418100ff03ff00020000000000000000000000000303d590")[..31]);
        let f = get_properties(&[0x39, 0x42, 0x0A, 0x09]);
        assert_eq!(f[..20], h("aa16ac00000000000003b104390042000a000900")[..]);
    }

    #[test]
    fn frame_checksums() {
        // Mesmo payload e ID da captura → mesmos CRC e checksum
        let payload = h("b5010001");
        assert_eq!(crc8(&payload), 0xE5);
        let f = h("aa0fac00000000000003b5010001e5a6");
        assert_eq!(checksum(&f[1..f.len() - 1]), 0xA6);
    }

    #[test]
    fn parses_state() {
        let r = Response::parse(&h("aa28ac00000000000203c00028657f7f003f00000067ff0c0009000000000000000100000000037da1")).unwrap();
        let Response::State(s) = r else { panic!("{r:?}") };
        assert!(!s.power);
        assert_eq!(s.mode, 1);
        assert_eq!(s.target_temperature, 24.0);
        assert_eq!(s.fan_speed, 101);
        assert_eq!(s.swing_mode, 15);
        assert!((s.indoor_temperature.unwrap() - 26.9).abs() < 1e-4);
        assert_eq!(s.outdoor_temperature, None);
        assert!(!s.eco && !s.turbo && !s.sleep && !s.purifier && !s.fahrenheit);
    }

    #[test]
    fn parses_capabilities() {
        let r1 = Response::parse(&h("aa31ac00000000000203b507120201011402010015020101160201001a02010110020101250207203c203c203c000100bfcf")).unwrap();
        let r2 = Response::parse(&h("aa2fac00000000000203b5081e0201011302010022020100190201003900010142000101090001010a00010100006de7")).unwrap();
        let (Response::Capabilities(mut c), Response::Capabilities(c2)) = (r1, r2) else { panic!() };
        assert!(c.additional);
        assert!(!c2.additional);
        c.merge(c2);
        for on in ["eco", "cool_mode", "dry_mode", "auto_mode", "swing_horizontal", "swing_vertical", "turbo_cool",
                   "fan_custom", "anion", "self_clean", "breeze_away", "swing_vertical_angle", "swing_horizontal_angle"] {
            assert!(c.get(on), "{on}");
        }
        for off in ["heat_mode", "freeze_protection", "fan_silent", "breeze_control", "display_control"] {
            assert!(!c.get(off), "{off}");
        }
        assert!(c.fan("silent") && c.fan("auto"));
        assert_eq!((c.min_temperature(), c.max_temperature()), (16.0, 30.0));
    }

    #[test]
    fn parses_properties() {
        let r = Response::parse(&h("aa22ac00000000000203b104390000010042000001010a0000010009000001000039ac")).unwrap();
        let Response::Properties(p) = r else { panic!() };
        assert_eq!(p.0.get(&PROP_SELF_CLEAN), Some(&0));
        assert_eq!(p.0.get(&PROP_BREEZE_AWAY), Some(&0));
        assert_eq!(p.0.get(&PROP_SWING_LR_ANGLE), Some(&0));
        assert_eq!(p.0.get(&PROP_SWING_UD_ANGLE), Some(&0));
    }

    #[test]
    fn set_state_encoding() {
        let s = SetState { power: true, target_temperature: 23.5, mode: 2, fan_speed: 102, swing_mode: 0xC, eco: true, target_humidity: 40, ..Default::default() };
        let f = s.encode();
        assert_eq!(f[9], FRAME_CONTROL);
        let d = &f[10..];
        assert_eq!(d[0], 0x40);
        assert_eq!(d[1], 0x03);
        assert_eq!(d[2], (23 - 16) | 0x10 | 2 << 5);
        assert_eq!(d[3], 102);
        assert_eq!(d[7], 0x3C);
        assert_eq!(d[9], 0x80);
        assert_eq!(d[19], 40);
        // O aparelho também aceita a resposta deste frame como válida
        assert_eq!(checksum(&f[1..f.len() - 1]), *f.last().unwrap());
        assert_eq!(crc8(&f[10..f.len() - 2]), f[f.len() - 2]);
    }
}
