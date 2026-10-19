//! Ar-condicionado Midea: painel + ícone na bandeja, num único processo.
//!
//! Usa o config.json da mesma pasta do executável; se ele não existir, abre a
//! tela de configuração, que o cria. Com `--bandeja` (usado ao iniciar com o
//! Windows) abre só o ícone da bandeja.

#![windows_subsystem = "windows"]

mod icon;
mod panel;
mod setup_view;
mod tray;

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use ar_midea::config::Config;
use ar_midea::controller::Controller;
use ar_midea::win::{self, wide};
use ar_midea::APP_NAME;
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// O aparelho derruba conexões paradas por ~30 s; consultando antes disso
/// a conexão fica aberta e cada leitura é rápida.
const POLL: Duration = Duration::from_secs(15);

fn main() {
    ar_midea::logging::init("ar.log");
    unsafe { SetCurrentProcessExplicitAppUserModelID(wide("ArCondicionadoMidea").as_ptr()) };
    let start_hidden = std::env::args().any(|a| a == win::TRAY_ARG);

    let Some(_mutex) = win::single_instance("Local\\ArCondicionadoMidea") else {
        // Já está rodando: pede para a outra instância mostrar o painel
        unsafe {
            let hwnd = FindWindowW(wide(tray::CLASS_NAME).as_ptr(), std::ptr::null());
            if !hwnd.is_null() && !start_hidden {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, &mut pid);
                AllowSetForegroundWindow(pid);
                PostMessageW(hwnd, tray::WM_SHOW_PANEL, 0, 0);
            }
        }
        return;
    };

    let cfg = Config::load().map_err(|e| log::warn!("Sem config.json válido ({e}); abrindo a configuração")).ok();
    // Sem configuração não há o que fazer na bandeja: mostra a tela de configuração
    let start_hidden = start_hidden && cfg.is_some();
    win::upgrade_autostart();
    log::info!("Iniciando ({})", if cfg.is_none() { "configuração" } else if start_hidden { "bandeja" } else { "painel" });

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_inner_size([460.0, initial_height()])
        .with_min_inner_size([420.0, 480.0]);
    if start_hidden {
        viewport = viewport.with_position(tray::OFFSCREEN);
    }
    let options = eframe::NativeOptions { viewport, centered: !start_hidden, ..Default::default() };
    let result = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            let hwnd = match cc.window_handle().map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(h)) => h.hwnd.get() as HWND,
                _ => return Err("janela sem HWND".into()),
            };
            set_window_icon(hwnd);
            let screen = match cfg {
                Some(cfg) => Screen::Panel(Box::new(start_panel(cfg, hwnd, &cc.egui_ctx, start_hidden)?)),
                None => Screen::Setup(setup_view::SetupView::new(&cc.egui_ctx)),
            };
            Ok(Box::new(Root { screen, hwnd }))
        }),
    );
    if let Err(e) = result {
        log::error!("Falha ao abrir a janela: {e}");
        win::error_box(&format!("Não foi possível abrir a janela:\n{e}"));
    }
}

/// Liga o controlador e a bandeja e devolve o painel.
fn start_panel(cfg: Config, hwnd: HWND, ctx: &egui::Context, start_hidden: bool) -> Result<panel::Panel, String> {
    // A bandeja precisa existir antes do controlador, que avisa as mudanças a ela
    let tray_hwnd = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let notify_hwnd = tray_hwnd.clone();
    let notify_ctx = ctx.clone();
    let ctl = Controller::start(cfg, POLL, move || {
        notify_ctx.request_repaint();
        let h = notify_hwnd.load(std::sync::atomic::Ordering::SeqCst);
        if h != 0 {
            unsafe { PostMessageW(h as HWND, tray::WM_UPDATE, 0, 0) };
        }
    })?;
    let ctl = Rc::new(ctl);
    let h = tray::create(hwnd, ctl.clone(), ctx.clone());
    tray_hwnd.store(h, std::sync::atomic::Ordering::SeqCst);
    Ok(panel::Panel::new(ctl, hwnd, start_hidden))
}

enum Screen {
    Setup(setup_view::SetupView),
    Panel(Box<panel::Panel>),
}

/// Começa na configuração (sem config.json) ou direto no painel.
struct Root {
    screen: Screen,
    hwnd: HWND,
}

impl eframe::App for Root {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        match &mut self.screen {
            Screen::Panel(p) => eframe::App::logic(p.as_mut(), ctx, frame),
            Screen::Setup(s) => {
                if let Some(cfg) = s.take_config() {
                    log::info!("Configuração gravada; abrindo o painel");
                    match start_panel(cfg, self.hwnd, ctx, false) {
                        Ok(p) => self.screen = Screen::Panel(Box::new(p)),
                        Err(e) => win::error_box(&format!("Não foi possível iniciar:\n{e}")),
                    }
                }
            }
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        match &self.screen {
            Screen::Panel(p) => eframe::App::clear_color(p.as_ref(), visuals),
            Screen::Setup(_) => [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        match &mut self.screen {
            Screen::Panel(p) => eframe::App::ui(p.as_mut(), ui, frame),
            Screen::Setup(s) => {
                let dark = ui.visuals().dark_mode;
                s.ui(ui, dark);
            }
        }
    }
}

/// 880 pontos, ou menos se a área de trabalho for mais baixa.
fn initial_height() -> f32 {
    let mut work = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // Antes do winit ligar o DPI por monitor, o valor vem em pontos lógicos
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0) } != 0;
    let available = if ok { (work.bottom - work.top) as f32 - 60.0 } else { 880.0 };
    available.clamp(480.0, 880.0)
}

fn set_window_icon(hwnd: HWND) {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        // Recurso 1 = app.ico (ver build.rs)
        for (which, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            let size = GetSystemMetrics(metric);
            let icon = LoadImageW(instance, 1 as _, IMAGE_ICON, size, size, LR_DEFAULTCOLOR);
            if !icon.is_null() {
                SendMessageW(hwnd, WM_SETICON, which as usize, icon as isize);
            }
        }
    }
}

/// Usa a Segoe UI do Windows; as fontes padrão do egui ficam de reserva.
fn setup_fonts(ctx: &egui::Context) {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let read = |f: &str| std::fs::read(format!(r"{dir}\Fonts\{f}")).ok();
    let mut fonts = egui::FontDefinitions::default();
    let fallback = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    let mut regular = fallback.clone();
    if let Some(data) = read("segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), Arc::new(egui::FontData::from_owned(data)));
        regular.insert(0, "segoe".into());
    }
    let mut bold = regular.clone();
    if let Some(data) = read("seguisb.ttf").or_else(|| read("segoeuib.ttf")) {
        fonts.font_data.insert("segoe-bold".into(), Arc::new(egui::FontData::from_owned(data)));
        bold.insert(0, "segoe-bold".into());
    }
    fonts.families.insert(egui::FontFamily::Proportional, regular);
    fonts.families.insert(egui::FontFamily::Name("bold".into()), bold);
    ctx.set_fonts(fonts);
}
