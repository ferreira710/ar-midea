//! Mantém a conexão com o aparelho numa thread própria.
//!
//! As interfaces leem um `Snapshot` e mandam alterações pelo `Controller`;
//! `on_change` é chamado (da thread do controlador) sempre que algo muda.

use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::device::{Capabilities, Device, State};
use crate::discover;
use crate::lan::Error;

type Change = Box<dyn FnOnce(&mut State) + Send>;

enum Cmd {
    Refresh,
    Change(Change),
    SelfClean,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub ready: bool,
    pub error: Option<String>,
    /// Alterações enviadas e ainda não aplicadas.
    pub pending: usize,
    /// Hora local da última leitura bem-sucedida ("HH:MM:SS").
    pub last_update: Option<String>,
    pub addr: SocketAddr,
    pub state: State,
    pub caps: Capabilities,
}

impl Snapshot {
    pub fn connected(&self) -> bool {
        self.ready && self.error.is_none()
    }
}

pub struct Controller {
    tx: Sender<Cmd>,
    shared: Arc<Mutex<Snapshot>>,
    pending: Arc<AtomicUsize>,
}

impl Controller {
    pub fn start(cfg: Config, poll: Duration, on_change: impl Fn() + Send + 'static) -> Result<Controller, String> {
        let addr = resolve(&cfg.host, cfg.port).ok_or_else(|| format!("endereço inválido: {}", cfg.host))?;
        let device = Device::new(addr, cfg.device_id, &cfg.token, &cfg.key).map_err(|e| e.to_string())?;
        let shared = Arc::new(Mutex::new(Snapshot {
            ready: false,
            error: None,
            pending: 0,
            last_update: None,
            addr,
            state: device.state.clone(),
            caps: device.caps.clone(),
        }));
        let pending = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel::<Cmd>();
        let mut worker = Worker { cfg, device, ready: false, shared: shared.clone(), pending: pending.clone(), on_change: Box::new(on_change) };
        std::thread::Builder::new()
            .name("controlador".into())
            .spawn(move || {
                worker.run(Cmd::Refresh);
                let mut next_poll = Instant::now() + poll;
                loop {
                    let wait = next_poll.saturating_duration_since(Instant::now());
                    let cmd = match rx.recv_timeout(wait) {
                        Ok(cmd) => cmd,
                        Err(RecvTimeoutError::Timeout) => Cmd::Refresh,
                        Err(RecvTimeoutError::Disconnected) => break,
                    };
                    worker.run(cmd);
                    next_poll = Instant::now() + poll;
                }
                worker.device.disconnect();
            })
            .map_err(|e| e.to_string())?;
        Ok(Controller { tx, shared, pending })
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner()).clone();
        s.pending = self.pending.load(Ordering::SeqCst);
        s
    }

    /// Altera o estado e envia ao aparelho. `f` roda na thread do controlador,
    /// sobre o estado mais recente.
    pub fn change(&self, f: impl FnOnce(&mut State) + Send + 'static) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        if self.tx.send(Cmd::Change(Box::new(f))).is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }

    pub fn refresh(&self) {
        let _ = self.tx.send(Cmd::Refresh);
    }

    pub fn start_self_clean(&self) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        if self.tx.send(Cmd::SelfClean).is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn resolve(host: &str, port: u16) -> Option<SocketAddr> {
    (host, port).to_socket_addrs().ok()?.find(|a| a.is_ipv4())
}

struct Worker {
    cfg: Config,
    device: Device,
    ready: bool,
    shared: Arc<Mutex<Snapshot>>,
    pending: Arc<AtomicUsize>,
    on_change: Box<dyn Fn() + Send>,
}

impl Worker {
    fn run(&mut self, cmd: Cmd) {
        let counts = !matches!(cmd, Cmd::Refresh);
        let mut before = None;
        let mut self_clean = false;
        match cmd {
            Cmd::Refresh => {}
            Cmd::SelfClean => self_clean = true,
            Cmd::Change(f) => {
                before = Some(self.device.state.clone());
                f(&mut self.device.state);
            }
        }
        let started = Instant::now();
        let mut result = self.attempt(before.as_ref(), self_clean);
        if result.is_err() {
            self.ready = false;
            if self.relocate() {
                result = self.attempt(before.as_ref(), self_clean);
            }
        }
        let mut snap = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(()) => {
                log::debug!("Comando concluído em {:?}", started.elapsed());
                snap.error = None;
                snap.last_update = Some(crate::win::local_time_hms());
            }
            Err(e) => {
                log::error!("Falha ao comunicar com o aparelho: {e}");
                snap.error = Some(e.to_string());
                self.ready = false;
                self.device.disconnect();
            }
        }
        snap.ready = self.ready;
        snap.addr = self.device.addr();
        snap.state = self.device.state.clone();
        snap.caps = self.device.caps.clone();
        drop(snap);
        if counts {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
        (self.on_change)();
    }

    fn attempt(&mut self, before: Option<&State>, self_clean: bool) -> Result<(), Error> {
        if !self.ready {
            self.device.authenticate()?;
            self.device.get_capabilities()?;
            self.ready = true;
        }
        if self_clean {
            self.device.start_self_clean()
        } else if let Some(before) = before {
            self.device.apply(before)
        } else {
            self.device.refresh()
        }
    }

    /// Procura o aparelho na rede pelo ID; se o IP mudou (DHCP), passa a usar
    /// o novo endereço e grava no config.json.
    fn relocate(&mut self) -> bool {
        let found = match discover::discover(Duration::from_secs(3)) {
            Ok(f) => f,
            Err(e) => {
                log::error!("Falha na busca do aparelho na rede: {e}");
                return false;
            }
        };
        let Some(dev) = found.into_iter().find(|d| d.id == self.device.id) else {
            return false;
        };
        if dev.addr == self.device.addr() {
            return false;
        }
        log::info!("Aparelho mudou de {} para {}", self.device.addr(), dev.addr);
        self.device.set_addr(dev.addr);
        self.cfg.host = dev.addr.ip().to_string();
        self.cfg.port = dev.addr.port();
        if let Err(e) = self.cfg.save() {
            log::error!("Não foi possível gravar {}: {e}", crate::config::path().display());
        }
        true
    }
}
