//! Log em arquivo ao lado do executável, com rotação simples (1 MB, um backup).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_BYTES: u64 = 1_000_000;

struct FileLogger {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

fn open(path: &PathBuf) -> Option<File> {
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn local_time() -> String {
    let t = crate::win::local_time();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02},{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

impl log::Log for FileLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let mut guard = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().and_then(|f| f.metadata().ok()).is_some_and(|m| m.len() > MAX_BYTES) {
            *guard = None;
            let backup = self.path.with_extension("log.1");
            let _ = std::fs::remove_file(&backup);
            let _ = std::fs::rename(&self.path, &backup);
            *guard = open(&self.path);
        }
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{} {} {}: {}", local_time(), r.level(), r.target(), r.args());
        }
    }

    fn flush(&self) {}
}

pub fn init(file_name: &str) {
    let path = crate::config::base_dir().join(file_name);
    let file = Mutex::new(open(&path));
    let logger = Box::leak(Box::new(FileLogger { path, file }));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    std::panic::set_hook(Box::new(|info| log::error!("Pânico: {info}")));
}
