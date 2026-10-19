//! Janela do painel (egui). Fechar a janela só a esconde na bandeja.

use std::rc::Rc;
use std::time::{Duration, Instant};

use ar_midea::controller::{Controller, Snapshot};
use ar_midea::device::{fan, Mode, State, Swing, SWING_ANGLES};
use ar_midea::{fmt_num, fmt_temp, mode_color, APP_NAME};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CursorIcon, FontFamily, FontId, Frame, Id, Margin, Rect, Sense, Ui, UiBuilder,
};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

const DEBOUNCE: Duration = Duration::from_millis(600);

const SHORT_MODES: [(Mode, &str); 5] = [
    (Mode::Auto, "Auto"),
    (Mode::Cool, "Frio"),
    (Mode::Dry, "Desumid."),
    (Mode::Heat, "Quente"),
    (Mode::FanOnly, "Ventilar"),
];
const SHORT_FANS: [(u8, &str); 6] = [
    (fan::AUTO, "Auto"),
    (fan::SILENT, "Silenc."),
    (fan::LOW, "Baixa"),
    (fan::MEDIUM, "Média"),
    (fan::HIGH, "Alta"),
    (fan::MAX, "Máx."),
];
const SHORT_SWINGS: [(Swing, &str); 4] =
    [(Swing::Off, "Parada"), (Swing::Vertical, "Vertical"), (Swing::Horizontal, "Horizontal"), (Swing::Both, "Ambas")];
const ANGLE_LABELS: [&str; 6] = ["Padrão", "1", "2", "3", "4", "5"];

fn long_mode(m: Mode) -> &'static str {
    match m {
        Mode::Auto => "Automático",
        Mode::Cool => "Resfriando",
        Mode::Dry | Mode::SmartDry => "Desumidificando",
        Mode::Heat => "Aquecendo",
        Mode::FanOnly => "Só ventilação",
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Feature {
    Eco,
    Turbo,
    Sleep,
    Purifier,
    BreezeAway,
}

const FEATURES: [(Feature, &str, &str); 5] = [
    (Feature::Eco, "Eco", "Economia de energia"),
    (Feature::Turbo, "Turbo", "Potência máxima por um tempo"),
    (Feature::Sleep, "Modo noturno", "Ajusta a temperatura aos poucos"),
    (Feature::Purifier, "Purificador", "Ionizador de ar"),
    (Feature::BreezeAway, "Vento indireto", "Evita jogar ar direto nas pessoas"),
];

impl Feature {
    fn get(self, s: &State) -> bool {
        match self {
            Feature::Eco => s.eco,
            Feature::Turbo => s.turbo,
            Feature::Sleep => s.sleep,
            Feature::Purifier => s.purifier,
            Feature::BreezeAway => s.breeze_away(),
        }
    }

    fn set(self, s: &mut State, on: bool) {
        match self {
            Feature::Eco => s.eco = on,
            Feature::Turbo => s.turbo = on,
            Feature::Sleep => s.sleep = on,
            Feature::Purifier => s.purifier = on,
            Feature::BreezeAway => s.set_breeze_away(on),
        }
    }

    fn supported(self, snap: &Snapshot) -> bool {
        match self {
            Feature::Eco => snap.caps.eco,
            Feature::Turbo => snap.caps.turbo,
            Feature::Sleep => true,
            Feature::Purifier => snap.caps.purifier,
            Feature::BreezeAway => snap.caps.breeze_away,
        }
    }
}

// ---------- cores ----------

fn hex(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

#[derive(Clone, Copy)]
struct Palette {
    bg: Color32,
    card: Color32,
    text: Color32,
    muted: Color32,
    track: Color32,
    track_hover: Color32,
    switch_off: Color32,
    ok: Color32,
    bad: Color32,
}

impl Palette {
    fn new(dark: bool) -> Palette {
        let pick = |light: u32, dark_c: u32| hex(if dark { dark_c } else { light });
        Palette {
            bg: pick(0xf2f3f7, 0x101217),
            card: pick(0xffffff, 0x1a1d24),
            text: pick(0x15171c, 0xeef0f4),
            muted: pick(0x6b7280, 0x8a92a3),
            track: pick(0xeceef3, 0x262a33),
            track_hover: pick(0xe0e3ea, 0x2f3440),
            switch_off: pick(0xd3d7df, 0x3a3f4b),
            ok: pick(0x2e7d32, 0x66bb6a),
            bad: pick(0xc62828, 0xef5350),
        }
    }
}

fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

// ---------- widgets ----------
//
// Os widgets desenham num retângulo já reservado; as linhas são montadas
// dividindo esse retângulo, sem depender do espaçamento automático do egui.

/// Reserva uma linha com toda a largura disponível.
fn row(ui: &mut Ui, height: f32) -> Rect {
    ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover()).0
}

/// Botão segmentado. Devolve a opção clicada.
#[allow(clippy::too_many_arguments)]
fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    options: &[(T, &str)],
    selected: Option<T>,
    accent: Color32,
    p: &Palette,
    font_id: FontId,
) -> Option<T> {
    let height = rect.height();
    let n = options.len().max(1);
    let pad = 3.0;
    let seg = (rect.width() - 2.0 * pad) / n as f32;
    let seg_rect = |i: f32| {
        Rect::from_min_size(pos2(rect.left() + pad + i * seg, rect.top() + pad), vec2(seg, height - 2.0 * pad))
    };
    let responses: Vec<_> = (0..n)
        .map(|i| ui.interact(seg_rect(i as f32), id.with(i), Sense::click()).on_hover_cursor(CursorIcon::PointingHand))
        .collect();
    let painter = ui.painter();
    painter.rect_filled(rect, 10, p.track);
    let index = selected.and_then(|s| options.iter().position(|(o, _)| *o == s));
    for (i, r) in responses.iter().enumerate() {
        if r.hovered() && Some(i) != index {
            painter.rect_filled(seg_rect(i as f32), 8, p.track_hover);
        }
    }
    let pos = index.map(|i| ui.ctx().animate_value_with_time(id.with("sel"), i as f32, 0.15));
    if let Some(x) = pos {
        painter.rect_filled(seg_rect(x), 8, accent);
    }
    for (i, (_, label)) in options.iter().enumerate() {
        // O texto fica branco conforme o destaque passa por cima
        let t = pos.map_or(0.0, |x| (1.0 - (x - i as f32).abs()).clamp(0.0, 1.0));
        painter.text(seg_rect(i as f32).center(), Align2::CENTER_CENTER, *label, font_id.clone(), mix(p.text, Color32::WHITE, t));
    }
    responses.iter().position(|r| r.clicked()).map(|i| options[i].0)
}

/// Chave liga/desliga. Devolve true se foi clicada.
fn switch(ui: &mut Ui, rect: Rect, id: Id, on: bool, accent: Color32, p: &Palette) -> bool {
    let resp = ui.interact(rect, id, Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    let t = ui.ctx().animate_bool_with_time(id, on, 0.12);
    let painter = ui.painter();
    painter.rect_filled(rect, 12, mix(p.switch_off, accent, t));
    let x = rect.left() + 12.0 + t * (rect.width() - 24.0);
    painter.circle_filled(pos2(x, rect.center().y), 9.0, Color32::WHITE);
    resp.clicked()
}

/// Barra de 1 a 100. Devolve o novo valor enquanto é arrastada.
fn slider(ui: &mut Ui, rect: Rect, id: Id, value: u8, accent: Color32, p: &Palette) -> Option<u8> {
    let resp = ui.interact(rect, id, Sense::click_and_drag()).on_hover_cursor(CursorIcon::PointingHand);
    let track = rect.shrink2(vec2(9.0, 0.0));
    let mut new = None;
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let t = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
            let v = (1.0 + t * 99.0).round() as u8;
            if v != value {
                new = Some(v);
            }
        }
    }
    let v = new.unwrap_or(value).clamp(1, 100);
    let t = (v as f32 - 1.0) / 99.0;
    let x = track.left() + t * track.width();
    let y = rect.center().y;
    let painter = ui.painter();
    let bar = Rect::from_min_max(pos2(track.left(), y - 3.0), pos2(track.right(), y + 3.0));
    painter.rect_filled(bar, 3, p.switch_off);
    painter.rect_filled(Rect::from_min_max(bar.min, pos2(x, bar.max.y)), 3, accent);
    let r = if resp.hovered() || resp.dragged() { 10.0 } else { 9.0 };
    painter.circle_filled(pos2(x, y), r, accent);
    new
}

struct ButtonStyle {
    fill: Color32,
    hover: Color32,
    text: Color32,
}

fn button(ui: &mut Ui, rect: Rect, id: Id, label: &str, font_id: FontId, radius: u8, s: &ButtonStyle) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    let enabled = ui.is_enabled();
    let resp = if enabled { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp };
    let fill = if resp.hovered() && enabled { s.hover } else { s.fill };
    ui.painter().rect_filled(rect, radius, fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font_id, s.text);
    resp.clicked()
}

/// Cartão branco/escuro com título opcional à esquerda e subtítulo à direita.
fn card<R>(ui: &mut Ui, fill: Color32, p: &Palette, title: Option<(&str, &str)>, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(fill)
        .corner_radius(16)
        .inner_margin(Margin { left: 18, right: 18, top: 14, bottom: 16 })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if let Some((title, subtitle)) = title {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
                ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, title, bold(15.0), p.text);
                ui.painter().text(rect.right_center(), Align2::RIGHT_CENTER, subtitle, font(12.0), p.muted);
                ui.add_space(8.0);
            }
            add(ui)
        })
        .inner
}

// ---------- painel ----------

pub struct Panel {
    ctl: Rc<Controller>,
    hwnd: HWND,
    /// Quadros até esconder a janela quando o app abre só na bandeja.
    hide_in_frames: Option<u8>,
    snap: Snapshot,
    /// Estado mostrado: o do aparelho mais as escolhas ainda não confirmadas.
    local: State,
    temp_pending: Option<(f32, Instant)>,
    fan_pending: Option<(u8, Instant)>,
    scroll_acc: f32,
    confirm_clean: bool,
}

impl Panel {
    pub fn new(ctl: Rc<Controller>, hwnd: HWND, start_hidden: bool) -> Panel {
        let snap = ctl.snapshot();
        Panel {
            local: snap.state.clone(),
            snap,
            ctl,
            hwnd,
            temp_pending: None,
            fan_pending: None,
            scroll_acc: 0.0,
            hide_in_frames: start_hidden.then_some(1),
            confirm_clean: false,
        }
    }

    /// Mostra a mudança na hora e envia ao aparelho.
    fn change(&mut self, f: impl Fn(&mut State) + Send + Clone + 'static) {
        f(&mut self.local);
        self.ctl.change(f);
    }

    fn sync(&mut self) {
        self.snap = self.ctl.snapshot();
        // Enquanto há alterações a caminho, mantém na tela a escolha do usuário
        if self.snap.pending == 0 {
            self.local = self.snap.state.clone();
        }
        let now = Instant::now();
        if let Some((t, at)) = self.temp_pending {
            if now >= at {
                self.temp_pending = None;
                self.change(move |s| s.target_temperature = t);
            }
        }
        if let Some((f, at)) = self.fan_pending {
            if now >= at {
                self.fan_pending = None;
                self.change(move |s| s.fan_speed = f);
            }
        }
    }

    fn step_temp(&mut self, delta: f32) {
        let c = &self.snap.caps;
        let base = self.temp_pending.map_or(self.local.target_temperature, |(t, _)| t);
        let t = (base + delta).clamp(c.min_temperature, c.max_temperature);
        self.temp_pending = Some((t, Instant::now() + DEBOUNCE));
    }

    fn header(&self, ui: &mut Ui, rect: Rect, p: &Palette) {
        let painter = ui.painter();
        painter.text(pos2(rect.left() + 20.0, rect.center().y), Align2::LEFT_CENTER, APP_NAME, bold(22.0), p.text);
        let (color, text) = if self.snap.error.is_some() {
            (p.bad, "Sem conexão".to_string())
        } else if !self.snap.ready {
            (p.muted, "Conectando…".to_string())
        } else {
            (p.ok, self.snap.addr.ip().to_string())
        };
        let r = painter.text(pos2(rect.right() - 20.0, rect.center().y), Align2::RIGHT_CENTER, &text, font(13.0), if self.snap.error.is_some() { p.bad } else { p.muted });
        painter.circle_filled(pos2(r.left() - 10.0, rect.center().y), 4.5, color);
    }

    fn footer(&mut self, ui: &mut Ui, rect: Rect, p: &Palette) {
        let text = match (&self.snap.error, &self.snap.last_update) {
            (Some(e), _) => format!("Erro: {e}").chars().take(60).collect(),
            (None, Some(t)) => format!("Atualizado às {t}"),
            _ => String::new(),
        };
        ui.painter().text(pos2(rect.left() + 20.0, rect.center().y), Align2::LEFT_CENTER, text, font(12.0), p.muted);
        let b = Rect::from_center_size(pos2(rect.right() - 20.0 - 48.0, rect.center().y), vec2(96.0, 30.0));
        let style = ButtonStyle { fill: p.track, hover: p.track_hover, text: p.text };
        if button(ui, b, Id::unique("refresh"), "Atualizar", font(12.0), 10, &style) {
            self.ctl.refresh();
        }
    }

    fn hero(&mut self, ui: &mut Ui, p: &Palette, accent: Color32) {
        let s = self.local.clone();
        let on = ui.ctx().animate_bool_with_time(Id::unique("power-anim"), s.power, 0.2);
        let white = Color32::WHITE;
        let fill = mix(p.card, accent, on);
        let text = mix(p.text, white, on);
        let muted = mix(p.muted, mix(accent, white, 0.75), on);
        let round = ButtonStyle {
            fill: mix(p.track, mix(accent, white, 0.18), on),
            hover: mix(p.track_hover, mix(accent, white, 0.28), on),
            text: mix(p.text, white, on),
        };
        let power = ButtonStyle {
            fill: mix(accent, white, on),
            hover: mix(mix(accent, Color32::BLACK, 0.15), mix(white, accent, 0.1), on),
            text: mix(white, accent, on),
        };
        card(ui, fill, p, None, |ui| {
            // Temperaturas interna e externa
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
            let painter = ui.painter();
            painter.text(row.left_top(), Align2::LEFT_TOP, "Ambiente", font(12.0), muted);
            painter.text(row.right_top(), Align2::RIGHT_TOP, "Externa", font(12.0), muted);
            painter.text(pos2(row.left(), row.bottom()), Align2::LEFT_BOTTOM, fmt_temp(s.indoor_temperature), bold(16.0), text);
            painter.text(pos2(row.right(), row.bottom()), Align2::RIGHT_BOTTOM, fmt_temp(s.outdoor_temperature), bold(16.0), text);

            // Temperatura desejada com − e +
            let (mid, _) = ui.allocate_exact_size(vec2(ui.available_width(), 104.0), Sense::hover());
            let c = mid.center();
            let shown = self.temp_pending.map_or(s.target_temperature, |(t, _)| t);
            let number = fmt_num(shown);
            let galley = ui.painter().layout_no_wrap(number.clone(), bold(76.0), text);
            let unit = ui.painter().layout_no_wrap("°C".into(), font(22.0), text);
            let total = galley.size().x + 2.0 + unit.size().x;
            let left = c.x - total / 2.0;
            let top = c.y - galley.size().y / 2.0;
            let temp_rect = Rect::from_min_size(pos2(left, top), vec2(total, galley.size().y));
            ui.painter().galley(temp_rect.min, galley, text);
            ui.painter().galley(pos2(left + total - unit.size().x, top + 20.0), unit, text);

            let fan_only = s.mode == Mode::FanOnly;
            let gap = (temp_rect.width() / 2.0 + 44.0).max(96.0);
            ui.add_enabled_ui(!fan_only, |ui| {
                for (dx, label, delta) in [(-gap, "−", -1.0), (gap, "+", 1.0)] {
                    let r = Rect::from_center_size(pos2(c.x + dx, c.y), vec2(56.0, 56.0));
                    if button(ui, r, Id::unique(("step", label)), label, bold(24.0), 28, &round) {
                        self.step_temp(delta);
                    }
                }
                // Roda do mouse sobre o número ajusta a temperatura
                let hover = ui.interact(temp_rect, Id::unique("temp-wheel"), Sense::hover());
                if hover.hovered() && ui.is_enabled() {
                    // Consome a rolagem para a página não rolar junto
                    let dy: f32 = ui.input_mut(|i| {
                        i.smooth_scroll_delta.y = 0.0;
                        i.events
                            .iter()
                            .filter_map(|e| match e {
                                egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                                _ => None,
                            })
                            .sum()
                    });
                    self.scroll_acc += dy;
                    if self.scroll_acc.abs() >= 1.0 {
                        self.step_temp(self.scroll_acc.signum());
                        self.scroll_acc = 0.0;
                    }
                }
            });

            let (lbl, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
            let state = format!("{} · {}", if s.power { "Ligado" } else { "Desligado" }, long_mode(s.mode));
            ui.painter().text(lbl.center(), Align2::CENTER_CENTER, state, font(13.0), muted);
            ui.add_space(10.0);

            let (btn, _) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::hover());
            if button(ui, btn, Id::unique("power"), if s.power { "Desligar" } else { "Ligar" }, bold(15.0), 14, &power) {
                let target = !s.power;
                self.change(move |s| s.power = target);
            }
        });
    }

    fn controls(&mut self, ui: &mut Ui, p: &Palette, accent: Color32) {
        let caps = self.snap.caps.clone();
        let s = self.local.clone();

        card(ui, p.card, p, Some(("Modo", "")), |ui| {
            let opts: Vec<_> = SHORT_MODES.iter().copied().filter(|(m, _)| caps.modes.contains(m)).collect();
            let r = row(ui, 36.0);
            if let Some(m) = segmented(ui, r, Id::unique("mode"), &opts, Some(s.mode), accent, p, font(13.0)) {
                self.change(move |s| s.mode = m);
            }
        });
        ui.add_space(12.0);

        let speed = self.fan_pending.map_or(s.fan_speed, |(f, _)| f);
        let subtitle = if speed > 100 { "automática".to_string() } else { format!("{speed}%") };
        card(ui, p.card, p, Some(("Ventilador", &subtitle)), |ui| {
            let opts: Vec<_> = SHORT_FANS.iter().copied().filter(|(f, _)| caps.fan_speeds.contains(f)).collect();
            let selected = opts.iter().map(|(f, _)| *f).find(|f| fan::matches(speed, *f));
            let r = row(ui, 36.0);
            if let Some(f) = segmented(ui, r, Id::unique("fan"), &opts, selected, accent, p, font(12.0)) {
                self.fan_pending = None;
                self.change(move |s| s.fan_speed = f);
            }
            if caps.custom_fan_speed {
                ui.add_space(10.0);
                let r = row(ui, 22.0);
                ui.painter().text(r.left_center(), Align2::LEFT_CENTER, "Ajuste fino", font(12.0), p.muted);
                let label = if speed > 100 { "auto".to_string() } else { format!("{speed}%") };
                ui.painter().text(r.right_center(), Align2::RIGHT_CENTER, label, font(12.0), p.text);
                let bar = Rect::from_min_max(pos2(r.left() + 80.0, r.top()), pos2(r.right() - 48.0, r.bottom()));
                if let Some(v) = slider(ui, bar, Id::unique("fan-slider"), speed.min(100), accent, p) {
                    self.fan_pending = Some((v, Instant::now() + DEBOUNCE));
                }
            }
        });
        ui.add_space(12.0);

        card(ui, p.card, p, Some(("Oscilação", "")), |ui| {
            let opts: Vec<_> = SHORT_SWINGS.iter().copied().filter(|(w, _)| caps.swing_modes.contains(w)).collect();
            let r = row(ui, 36.0);
            if let Some(w) = segmented(ui, r, Id::unique("swing"), &opts, Some(s.swing), accent, p, font(13.0)) {
                self.change(move |s| s.swing = w);
            }
            let angles: Vec<_> = SWING_ANGLES.iter().copied().zip(ANGLE_LABELS).collect();
            for (vertical, title, supported) in
                [(true, "Aleta vertical", caps.vertical_swing_angle), (false, "Aleta horizontal", caps.horizontal_swing_angle)]
            {
                if !supported {
                    continue;
                }
                ui.add_space(12.0);
                let r = row(ui, 30.0);
                ui.painter().text(r.left_center(), Align2::LEFT_CENTER, title, font(12.0), p.muted);
                let current = if vertical { s.vertical_swing_angle } else { s.horizontal_swing_angle };
                let seg = Rect::from_min_max(pos2(r.left() + 104.0, r.top()), r.max);
                if let Some(a) = segmented(ui, seg, Id::unique(title), &angles, Some(current), accent, p, font(12.0)) {
                    if vertical {
                        self.change(move |s| s.vertical_swing_angle = a);
                    } else {
                        self.change(move |s| s.horizontal_swing_angle = a);
                    }
                }
            }
        });
        ui.add_space(12.0);

        card(ui, p.card, p, Some(("Funções", "")), |ui| {
            let mut first = true;
            for (feature, title, desc) in FEATURES {
                if !feature.supported(&self.snap) {
                    continue;
                }
                if !first {
                    ui.add_space(12.0);
                }
                first = false;
                let r = row(ui, 38.0);
                ui.painter().text(r.left_top(), Align2::LEFT_TOP, title, font(13.0), p.text);
                ui.painter().text(r.left_bottom(), Align2::LEFT_BOTTOM, desc, font(12.0), p.muted);
                let sw = Rect::from_center_size(pos2(r.right() - 22.0, r.center().y), vec2(44.0, 24.0));
                let on = feature.get(&s);
                if switch(ui, sw, Id::unique(("feature", title)), on, accent, p) {
                    self.change(move |s| feature.set(s, !on));
                }
            }
        });

        if caps.self_clean {
            ui.add_space(12.0);
            card(ui, p.card, p, Some(("Manutenção", "")), |ui| {
                {
                    let r = row(ui, 38.0);
                    let desc = if s.self_clean_active {
                        "Limpeza em andamento…"
                    } else {
                        "Seca e limpa o evaporador (leva alguns minutos)"
                    };
                    ui.painter().text(r.left_top(), Align2::LEFT_TOP, "Autolimpeza", font(13.0), p.text);
                    ui.painter().text(r.left_bottom(), Align2::LEFT_BOTTOM, desc, font(12.0), p.muted);
                    let b = Rect::from_center_size(pos2(r.right() - 45.0, r.center().y), vec2(90.0, 34.0));
                    let style = ButtonStyle { fill: p.track, hover: p.track_hover, text: p.text };
                    ui.add_enabled_ui(!s.self_clean_active, |ui| {
                        if button(ui, b, Id::unique("clean"), "Iniciar", font(13.0), 10, &style) {
                            self.confirm_clean = true;
                        }
                    });
                }
            });
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.confirm_clean {
            return;
        }
        let frame = Frame::new().fill(p.card).corner_radius(16).inner_margin(Margin::same(20));
        let resp = egui::Modal::new(Id::unique("confirm-clean")).frame(frame).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.label(egui::RichText::new("Iniciar a autolimpeza agora?").font(bold(15.0)).color(p.text));
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("O aparelho fica alguns minutos limpando e depois desliga sozinho.")
                    .font(font(13.0))
                    .color(p.muted),
            );
            ui.add_space(16.0);
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
            let accent = rgb(mode_color(self.local.mode));
            let start = ButtonStyle { fill: accent, hover: mix(accent, Color32::BLACK, 0.15), text: Color32::WHITE };
            let cancel = ButtonStyle { fill: p.track, hover: p.track_hover, text: p.text };
            let r_start = Rect::from_min_size(pos2(row.right() - 96.0, row.top()), vec2(96.0, 34.0));
            let r_cancel = Rect::from_min_size(pos2(row.right() - 200.0, row.top()), vec2(96.0, 34.0));
            if button(ui, r_start, Id::unique("clean-yes"), "Iniciar", font(13.0), 10, &start) {
                self.ctl.start_self_clean();
                self.confirm_clean = false;
            }
            if button(ui, r_cancel, Id::unique("clean-no"), "Cancelar", font(13.0), 10, &cancel) {
                self.confirm_clean = false;
            }
        });
        if resp.should_close() {
            self.confirm_clean = false;
        }
    }
}

impl eframe::App for Panel {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // O eframe sempre mostra a janela após o primeiro quadro; ela nasce fora
        // da tela e é escondida no quadro seguinte
        match self.hide_in_frames {
            Some(0) => {
                unsafe { ShowWindow(self.hwnd, SW_HIDE) };
                self.hide_in_frames = None;
            }
            Some(n) => {
                self.hide_in_frames = Some(n - 1);
                ctx.request_repaint();
            }
            None => {}
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            // O X só esconde a janela; o app continua na bandeja
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
        }
        self.sync();
        let next = [self.temp_pending.map(|(_, at)| at), self.fan_pending.map(|(_, at)| at)].into_iter().flatten().min();
        if let Some(at) = next {
            ctx.request_repaint_after(at.saturating_duration_since(Instant::now()));
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        Palette::new(visuals.dark_mode).bg.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let p = Palette::new(ui.visuals().dark_mode);
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0, p.bg);
        let accent = rgb(mode_color(self.local.mode));
        let connected = self.snap.connected();

        let header = Rect::from_min_size(full.min, vec2(full.width(), 60.0));
        let footer = Rect::from_min_max(pos2(full.left(), full.bottom() - 54.0), full.max);
        let body = Rect::from_min_max(pos2(full.left(), header.bottom()), pos2(full.right(), footer.top()));

        self.header(ui, header, &p);
        ui.scope_builder(UiBuilder::new().max_rect(footer), |ui| self.footer(ui, footer, &p));
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                Frame::new().inner_margin(Margin { left: 16, right: 16, top: 0, bottom: 8 }).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                    ui.add_enabled_ui(connected, |ui| {
                        self.hero(ui, &p, accent);
                        ui.add_space(12.0);
                        self.controls(ui, &p, accent);
                    });
                });
            });
        });
        self.confirm_dialog(ui.ctx(), &p);
    }
}
