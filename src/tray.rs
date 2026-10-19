//! Ícone e menu da bandeja (Win32 direto). Roda na thread da interface; as
//! mensagens chegam pelo mesmo loop de mensagens do winit.

use std::cell::RefCell;
use std::rc::Rc;

use ab_glyph::FontVec;
use ar_midea::controller::{Controller, Snapshot};
use ar_midea::device::{fan, Mode, Swing};
use ar_midea::win::wide;
use ar_midea::{fmt_num, fmt_temp, mode_color, mode_label, APP_NAME, ERROR_COLOR, FAN_SPEEDS, MODES, OFF_COLOR, SWING_MODES};
use eframe::egui;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const WM_TRAY: u32 = WM_APP + 1;
pub const WM_UPDATE: u32 = WM_APP + 2;
/// Enviada por uma segunda instância para mostrar o painel.
pub const WM_SHOW_PANEL: u32 = WM_APP + 3;
pub const CLASS_NAME: &str = "ArCondicionadoMideaBandeja";

#[derive(Clone, Copy)]
enum Action {
    OpenPanel,
    TogglePower,
    Step(f32),
    Temp(f32),
    Mode(Mode),
    Fan(u8),
    Swing(Swing),
    Eco,
    Turbo,
    Sleep,
    Purifier,
    Refresh,
    Autostart,
    Quit,
}

struct Tray {
    hwnd: HWND,
    panel: HWND,
    ctl: Rc<Controller>,
    ctx: egui::Context,
    font: Option<FontVec>,
    icon: HICON,
    last_error: Option<String>,
    taskbar_created: u32,
}

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

fn with_tray<R>(f: impl FnOnce(&mut Tray) -> R) -> Option<R> {
    TRAY.with(|t| t.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Cria a janela oculta e o ícone. Devolve o HWND (como usize, para mandar entre threads).
pub fn create(panel: HWND, ctl: Rc<Controller>, ctx: egui::Context) -> usize {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide(CLASS_NAME);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("").as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        TRAY.with(|t| {
            *t.borrow_mut() = Some(Tray {
                hwnd,
                panel,
                ctl,
                ctx,
                font: crate::icon::load_font(),
                icon: std::ptr::null_mut(),
                last_error: None,
                taskbar_created,
            })
        });
        with_tray(|t| t.add_icon());
        hwnd as usize
    }
}

/// Posição inicial da janela quando o app abre só na bandeja.
pub const OFFSCREEN: [f32; 2] = [-20000.0, -20000.0];

pub fn show_panel(panel: HWND, ctx: &egui::Context) {
    unsafe {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetWindowRect(panel, &mut r);
        if r.left < -10000 {
            let mut work = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let x = work.left + (work.right - work.left - w) / 2;
            let y = work.top + ((work.bottom - work.top - h) / 2).max(0);
            SetWindowPos(panel, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER);
        }
        ShowWindow(panel, if IsIconic(panel) != 0 { SW_RESTORE } else { SW_SHOW });
        SetForegroundWindow(panel);
    }
    ctx.request_repaint();
}

fn notify_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid
}

fn copy_into(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

fn status(s: &Snapshot) -> String {
    if let Some(e) = &s.error {
        return format!("Erro: {e}");
    }
    if !s.ready {
        return "Conectando...".into();
    }
    let ambient = format!("ambiente {}", fmt_temp(s.state.indoor_temperature));
    if !s.state.power {
        return format!("Desligado · {ambient}");
    }
    format!("{} · {} · {ambient}", mode_label(s.state.mode), fmt_temp(Some(s.state.target_temperature)))
}

impl Tray {
    fn render_icon(&mut self, s: &Snapshot) -> HICON {
        let (color, text) = if s.error.is_some() {
            (ERROR_COLOR, "!".to_string())
        } else if !s.ready {
            (OFF_COLOR, "…".to_string())
        } else {
            let color = if s.state.power { mode_color(s.state.mode) } else { OFF_COLOR };
            (color, format!("{:.0}", s.state.target_temperature))
        };
        let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) }.max(16) as u32;
        let rgba = crate::icon::render(size, color, &text, self.font.as_ref());
        crate::icon::to_hicon(&rgba, size)
    }

    fn add_icon(&mut self) {
        let snap = self.ctl.snapshot();
        let mut nid = notify_data(self.hwnd);
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        let icon = self.render_icon(&snap);
        self.replace_icon(icon);
        nid.hIcon = self.icon;
        copy_into(&mut nid.szTip, &format!("{APP_NAME}: {}", status(&snap)));
        unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
    }

    fn replace_icon(&mut self, icon: HICON) {
        let old = std::mem::replace(&mut self.icon, icon);
        if !old.is_null() {
            // O shell já copiou o ícone antigo; libera depois do NIM_MODIFY
            unsafe { DestroyIcon(old) };
        }
    }

    fn update(&mut self) {
        let snap = self.ctl.snapshot();
        let mut nid = notify_data(self.hwnd);
        nid.uFlags = NIF_ICON | NIF_TIP;
        let icon = self.render_icon(&snap);
        nid.hIcon = icon;
        copy_into(&mut nid.szTip, &format!("{APP_NAME}: {}", status(&snap)));
        if snap.error.is_some() && snap.error != self.last_error {
            nid.uFlags |= NIF_INFO;
            nid.dwInfoFlags = NIIF_WARNING;
            copy_into(&mut nid.szInfoTitle, APP_NAME);
            copy_into(&mut nid.szInfo, &format!("Não foi possível falar com o aparelho: {}", snap.error.as_deref().unwrap_or("")));
        }
        self.last_error = snap.error.clone();
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
        self.replace_icon(icon);
    }

    fn remove_icon(&mut self) {
        let nid = notify_data(self.hwnd);
        unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
    }

    fn build_menu(&self) -> (HMENU, Vec<Action>) {
        let snap = self.ctl.snapshot();
        let (s, c) = (&snap.state, &snap.caps);
        let on = snap.connected();
        let mut actions = Vec::new();
        let menu = unsafe { CreatePopupMenu() };
        let mut item = |menu: HMENU, label: &str, action: Option<Action>, checked: bool, radio: bool, enabled: bool| {
            let id = match action {
                Some(a) => {
                    actions.push(a);
                    actions.len() as u32
                }
                None => 0,
            };
            add_item(menu, label, id, checked, radio, enabled && action.is_some());
        };

        item(menu, &status(&snap), None, false, false, false);
        item(menu, "Abrir painel", Some(Action::OpenPanel), false, false, true);
        separator(menu);
        item(menu, if s.power { "Desligar" } else { "Ligar" }, Some(Action::TogglePower), false, false, on);
        item(menu, "Aumentar 1 °C", Some(Action::Step(1.0)), false, false, on);
        item(menu, "Diminuir 1 °C", Some(Action::Step(-1.0)), false, false, on);

        let temps = unsafe { CreatePopupMenu() };
        for t in 16..=30 {
            let t = t as f32;
            if (c.min_temperature..=c.max_temperature).contains(&t) {
                item(temps, &format!("{} °C", fmt_num(t)), Some(Action::Temp(t)), s.target_temperature == t, true, true);
            }
        }
        add_submenu(menu, "Temperatura", temps, on);

        let modes = unsafe { CreatePopupMenu() };
        for (m, label) in MODES.iter().filter(|(m, _)| c.modes.contains(m)) {
            item(modes, label, Some(Action::Mode(*m)), s.mode == *m, true, true);
        }
        add_submenu(menu, "Modo", modes, on);

        let fans = unsafe { CreatePopupMenu() };
        for (f, label) in FAN_SPEEDS.iter().filter(|(f, _)| c.fan_speeds.contains(f)) {
            item(fans, label, Some(Action::Fan(*f)), fan::matches(s.fan_speed, *f), true, true);
        }
        add_submenu(menu, "Ventilador", fans, on);

        let swings = unsafe { CreatePopupMenu() };
        for (w, label) in SWING_MODES.iter().filter(|(w, _)| c.swing_modes.contains(w)) {
            item(swings, label, Some(Action::Swing(*w)), s.swing == *w, true, true);
        }
        add_submenu(menu, "Oscilação", swings, on);

        separator(menu);
        if c.eco {
            item(menu, "Eco", Some(Action::Eco), s.eco, false, on);
        }
        if c.turbo {
            item(menu, "Turbo", Some(Action::Turbo), s.turbo, false, on);
        }
        item(menu, "Modo noturno", Some(Action::Sleep), s.sleep, false, on);
        if c.purifier {
            item(menu, "Purificador", Some(Action::Purifier), s.purifier, false, on);
        }
        separator(menu);
        item(menu, "Atualizar agora", Some(Action::Refresh), false, false, true);
        item(menu, "Iniciar com o Windows", Some(Action::Autostart), ar_midea::win::autostart_enabled(), false, true);
        item(menu, "Sair", Some(Action::Quit), false, false, true);
        if let Some(i) = actions.iter().position(|a| matches!(a, Action::TogglePower)) {
            unsafe { SetMenuDefaultItem(menu, i as u32 + 1, 0) };
        }
        (menu, actions)
    }

    fn run(&mut self, action: Action) {
        let ctl = &self.ctl;
        match action {
            Action::OpenPanel => show_panel(self.panel, &self.ctx),
            Action::TogglePower => ctl.change(|s| s.power = !s.power),
            Action::Step(delta) => {
                let caps = ctl.snapshot().caps;
                ctl.change(move |s| {
                    s.target_temperature = (s.target_temperature + delta).clamp(caps.min_temperature, caps.max_temperature)
                });
            }
            Action::Temp(t) => ctl.change(move |s| s.target_temperature = t),
            Action::Mode(m) => ctl.change(move |s| s.mode = m),
            Action::Fan(f) => ctl.change(move |s| s.fan_speed = f),
            Action::Swing(w) => ctl.change(move |s| s.swing = w),
            Action::Eco => ctl.change(|s| s.eco = !s.eco),
            Action::Turbo => ctl.change(|s| s.turbo = !s.turbo),
            Action::Sleep => ctl.change(|s| s.sleep = !s.sleep),
            Action::Purifier => ctl.change(|s| s.purifier = !s.purifier),
            Action::Refresh => ctl.refresh(),
            Action::Autostart => ar_midea::win::set_autostart(!ar_midea::win::autostart_enabled()),
            Action::Quit => {
                self.remove_icon();
                log::info!("Encerrando");
                std::process::exit(0);
            }
        }
        // Mostra a escolha no painel na hora
        self.ctx.request_repaint();
    }
}

fn add_item(menu: HMENU, label: &str, id: u32, checked: bool, radio: bool, enabled: bool) {
    let text = wide(label);
    let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MENUITEMINFOW>() as u32;
    info.fMask = MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_STRING;
    info.fType = MFT_STRING | if radio { MFT_RADIOCHECK } else { 0 };
    info.fState = if checked { MFS_CHECKED } else { 0 } | if enabled { 0 } else { MFS_DISABLED };
    info.wID = id;
    info.dwTypeData = text.as_ptr() as *mut u16;
    unsafe { InsertMenuItemW(menu, u32::MAX, 1, &info) };
}

fn add_submenu(menu: HMENU, label: &str, sub: HMENU, enabled: bool) {
    let flags = MF_POPUP | MF_STRING | if enabled { 0 } else { MF_GRAYED };
    unsafe { AppendMenuW(menu, flags, sub as usize, wide(label).as_ptr()) };
}

fn separator(menu: HMENU) {
    unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null()) };
}

fn show_menu(hwnd: HWND) {
    let Some((menu, actions)) = with_tray(|t| t.build_menu()) else { return };
    // TrackPopupMenu tem seu próprio loop de mensagens: não pode segurar o RefCell aqui
    let chosen = unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, hwnd, std::ptr::null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        id
    };
    if chosen > 0 {
        if let Some(&action) = actions.get(chosen as usize - 1) {
            with_tray(|t| t.run(action));
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            match lparam as u32 {
                WM_LBUTTONUP => {
                    with_tray(|t| show_panel(t.panel, &t.ctx));
                }
                WM_RBUTTONUP => show_menu(hwnd),
                _ => {}
            }
            0
        }
        WM_UPDATE => {
            with_tray(|t| t.update());
            0
        }
        WM_SHOW_PANEL => {
            with_tray(|t| show_panel(t.panel, &t.ctx));
            0
        }
        _ => {
            let readd = with_tray(|t| msg == t.taskbar_created).unwrap_or(false);
            if readd {
                // O Explorer reiniciou: o ícone precisa ser recriado
                with_tray(|t| t.add_icon());
                return 0;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}
