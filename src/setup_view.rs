//! Tela de configuração, mostrada quando não há config.json: procura o aparelho
//! na rede, obtém a chave pela nuvem da Midea e grava o arquivo.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ar_midea::config::{self, Config};
use ar_midea::discover::{self, Found};
use ar_midea::setup::{self, Account};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontFamily, FontId, Frame, Margin, RichText, Sense, Ui};

const AC_TYPE: u8 = 0xAC;

#[derive(Default)]
struct Shared {
    searching: bool,
    found: Vec<Found>,
    search_error: Option<String>,
    working: bool,
    log: Vec<String>,
    result: Option<Result<Config, String>>,
}

pub struct SetupView {
    shared: Arc<Mutex<Shared>>,
    selected: Option<u64>,
    email: String,
    password: String,
}

fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

impl SetupView {
    pub fn new(ctx: &egui::Context) -> SetupView {
        let view = SetupView { shared: Arc::default(), selected: None, email: String::new(), password: String::new() };
        view.search(ctx);
        view
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn search(&self, ctx: &egui::Context) {
        log::info!("Procurando aparelhos na rede");
        {
            let mut s = self.lock();
            s.searching = true;
            s.search_error = None;
        }
        let (shared, ctx) = (self.shared.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = discover::discover(Duration::from_secs(3));
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.searching = false;
            match result {
                Ok(found) => s.found = found.into_iter().filter(|d| d.device_type == Some(AC_TYPE)).collect(),
                Err(e) => s.search_error = Some(e.to_string()),
            }
            ctx.request_repaint();
        });
    }

    fn obtain(&self, ctx: &egui::Context, dev: Found) {
        let account = (!self.email.trim().is_empty())
            .then(|| Account { email: self.email.trim().to_string(), password: self.password.clone() });
        {
            let mut s = self.lock();
            s.working = true;
            s.log.clear();
            s.result = None;
        }
        let (shared, ctx) = (self.shared.clone(), ctx.clone());
        std::thread::spawn(move || {
            let progress = {
                let (shared, ctx) = (shared.clone(), ctx.clone());
                move |msg: String| {
                    shared.lock().unwrap_or_else(|e| e.into_inner()).log.push(msg);
                    ctx.request_repaint();
                }
            };
            let mut result = setup::obtain(&dev, account.as_ref(), progress);
            if let Ok(cfg) = &mut result {
                if let Err(e) = cfg.save() {
                    result = Err(format!("não foi possível gravar {}: {e}", config::path().display()));
                }
            }
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.working = false;
            s.result = Some(result);
            ctx.request_repaint();
        });
    }

    /// Devolve a configuração quando ela foi obtida e gravada.
    pub fn take_config(&mut self) -> Option<Config> {
        let mut s = self.lock();
        match s.result.take() {
            Some(Ok(cfg)) => Some(cfg),
            other => {
                s.result = other;
                None
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, dark: bool) {
        let pick = |light: u32, dark_c: u32| {
            let c = if dark { dark_c } else { light };
            Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
        };
        let (bg, card, text, muted, track) =
            (pick(0xf2f3f7, 0x101217), pick(0xffffff, 0x1a1d24), pick(0x15171c, 0xeef0f4), pick(0x6b7280, 0x8a92a3), pick(0xeceef3, 0x262a33));
        let accent = Color32::from_rgb(21, 101, 192);
        let bad = pick(0xc62828, 0xef5350);
        ui.painter().rect_filled(ui.max_rect(), 0, bg);

        let (searching, found, search_error, working, log, result_err) = {
            let s = self.lock();
            let err = match &s.result {
                Some(Err(e)) => Some(e.clone()),
                _ => None,
            };
            (s.searching, s.found.clone(), s.search_error.clone(), s.working, s.log.clone(), err)
        };
        if self.selected.is_none() && found.len() == 1 {
            self.selected = Some(found[0].id);
        }

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            Frame::new().inner_margin(Margin { left: 16, right: 16, top: 18, bottom: 16 }).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                ui.label(RichText::new("Configurar o aparelho").font(bold(22.0)).color(text));
                ui.label(
                    RichText::new("Não encontrei o config.json. Vou procurar o ar-condicionado na rede e buscar a chave de acesso dele na nuvem da Midea. Isso só é preciso uma vez.")
                        .font(font(13.0))
                        .color(muted),
                );
                ui.add_space(10.0);

                let card_frame = || Frame::new().fill(card).corner_radius(16).inner_margin(Margin::symmetric(18, 14));
                card_frame().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(RichText::new("1. Aparelho").font(bold(15.0)).color(text));
                    if searching {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Procurando na rede…").font(font(13.0)).color(muted));
                        });
                    } else if let Some(e) = &search_error {
                        ui.label(RichText::new(format!("Erro na busca: {e}")).font(font(13.0)).color(bad));
                    } else if found.is_empty() {
                        ui.label(
                            RichText::new("Nenhum ar-condicionado respondeu. Confira se ele está ligado na tomada e no mesmo Wi-Fi.")
                                .font(font(13.0))
                                .color(muted),
                        );
                    }
                    for dev in &found {
                        let selected = self.selected == Some(dev.id);
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
                        let fill = if selected { accent } else if resp.hovered() { track } else { card };
                        ui.painter().rect_filled(rect, 10, fill);
                        if !selected {
                            ui.painter().rect_stroke(rect, 10, (1.0, track), egui::StrokeKind::Inside);
                        }
                        let fg = if selected { Color32::WHITE } else { text };
                        ui.painter().text(pos2(rect.left() + 14.0, rect.center().y - 8.0), Align2::LEFT_CENTER, format!("Ar-condicionado em {}", dev.addr.ip()), font(13.0), fg);
                        let sub = if selected { Color32::from_white_alpha(200) } else { muted };
                        ui.painter().text(pos2(rect.left() + 14.0, rect.center().y + 9.0), Align2::LEFT_CENTER, format!("ID {} · protocolo V{}", dev.id, dev.version), font(11.5), sub);
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.selected = Some(dev.id);
                        }
                    }
                    if !searching && ui.add_enabled(!working, egui::Button::new(RichText::new("Procurar de novo").font(font(12.0)))).clicked() {
                        self.search(ui.ctx());
                    }
                });
                ui.add_space(10.0);

                card_frame().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(RichText::new("2. Conta Midea").font(bold(15.0)).color(text));
                    ui.label(
                        RichText::new("Use a conta do app (MSmartHome ou NetHome Plus) em que o ar foi cadastrado. Ela só é usada agora para buscar a chave; não fica gravada.")
                            .font(font(12.0))
                            .color(muted),
                    );
                    ui.add_space(4.0);
                    ui.add_enabled_ui(!working, |ui| {
                        ui.label(RichText::new("E-mail").font(font(12.0)).color(muted));
                        ui.add(egui::TextEdit::singleline(&mut self.email).font(font(14.0)).desired_width(f32::INFINITY).margin(vec2(8.0, 6.0)));
                        ui.label(RichText::new("Senha").font(font(12.0)).color(muted));
                        ui.add(egui::TextEdit::singleline(&mut self.password).password(true).font(font(14.0)).desired_width(f32::INFINITY).margin(vec2(8.0, 6.0)));
                    });
                });
                ui.add_space(14.0);

                let dev = found.iter().find(|d| Some(d.id) == self.selected).cloned();
                let enabled = dev.is_some() && !working && !searching;
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
                let fill = if !enabled { track } else if resp.hovered() { Color32::from_rgb(17, 86, 166) } else { accent };
                ui.painter().rect_filled(rect, 14, fill);
                let label = if working { "Obtendo a chave…" } else { "Obter chave e conectar" };
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, bold(15.0), if enabled { Color32::WHITE } else { muted });
                if enabled && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    if let Some(dev) = dev {
                        self.obtain(ui.ctx(), dev);
                    }
                }

                if let Some(e) = &result_err {
                    ui.add_space(8.0);
                    let hint = if self.email.trim().is_empty() { " As contas públicas não têm mais acesso; informe a sua conta Midea." } else { "" };
                    ui.label(RichText::new(format!("{e}{hint}")).font(font(13.0)).color(bad));
                }
                if !log.is_empty() {
                    ui.add_space(8.0);
                    Frame::new().fill(card).corner_radius(12).inner_margin(Margin::same(12)).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        egui::ScrollArea::vertical().id_salt("setup-log").max_height(240.0).stick_to_bottom(true).show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            for line in &log {
                                ui.label(RichText::new(line).font(font(11.5)).color(muted));
                            }
                        });
                    });
                }
            });
        });
    }
}
