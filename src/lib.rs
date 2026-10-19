//! Controle local do ar-condicionado Midea (dongle EU-SK105, protocolo V3).
//!
//! Biblioteca usada pelo ArCondicionado.exe: protocolo do aparelho, busca na
//! rede, obtenção da chave pela nuvem e o controlador em segundo plano.

pub mod ac;
pub mod cloud;
pub mod config;
pub mod controller;
pub mod device;
pub mod discover;
pub mod lan;
pub mod logging;
pub mod setup;
pub mod win;

use device::{fan, Mode, Swing};

pub const APP_NAME: &str = "Ar-condicionado";

pub const MODES: [(Mode, &str); 5] = [
    (Mode::Auto, "Automático"),
    (Mode::Cool, "Frio"),
    (Mode::Dry, "Desumidificar"),
    (Mode::Heat, "Quente"),
    (Mode::FanOnly, "Ventilar"),
];

pub const FAN_SPEEDS: [(u8, &str); 6] = [
    (fan::AUTO, "Automática"),
    (fan::SILENT, "Silenciosa"),
    (fan::LOW, "Baixa"),
    (fan::MEDIUM, "Média"),
    (fan::HIGH, "Alta"),
    (fan::MAX, "Máxima"),
];

pub const SWING_MODES: [(Swing, &str); 4] = [
    (Swing::Off, "Desligada"),
    (Swing::Vertical, "Vertical"),
    (Swing::Horizontal, "Horizontal"),
    (Swing::Both, "Vertical e horizontal"),
];

pub const OFF_COLOR: [u8; 3] = [110, 110, 110];
pub const ERROR_COLOR: [u8; 3] = [183, 28, 28];

pub fn mode_label(mode: Mode) -> &'static str {
    MODES.iter().find(|(m, _)| *m == mode).map_or("Desumidificar", |(_, l)| l)
}

pub fn mode_color(mode: Mode) -> [u8; 3] {
    match mode {
        Mode::Auto => [46, 125, 50],
        Mode::Cool => [21, 101, 192],
        Mode::Dry | Mode::SmartDry => [0, 131, 143],
        Mode::Heat => [230, 81, 0],
        Mode::FanOnly => [94, 53, 177],
    }
}

/// "24 °C", "23,5 °C" ou "-- °C".
pub fn fmt_temp(t: Option<f32>) -> String {
    match t {
        Some(t) => format!("{} °C", fmt_num(t)),
        None => "-- °C".into(),
    }
}

/// Número com vírgula decimal e sem zeros à direita ("24", "26,9").
pub fn fmt_num(t: f32) -> String {
    let s = format!("{:.1}", t);
    s.trim_end_matches('0').trim_end_matches('.').replace('.', ",")
}

#[cfg(test)]
mod tests {
    #[test]
    fn number_format() {
        assert_eq!(super::fmt_num(24.0), "24");
        assert_eq!(super::fmt_num(26.9), "26,9");
        assert_eq!(super::fmt_temp(Some(23.5)), "23,5 °C");
        assert_eq!(super::fmt_temp(None), "-- °C");
    }
}
