//! Diagnóstico somente leitura: conecta, lê capacidades e estado.
//! Uso: cargo run --example probe [segundos de espera entre leituras]
use std::time::{Duration, Instant};

use ar_midea::config::Config;
use ar_midea::device::Device;

fn main() {
    let cfg = Config::load().expect("config.json");
    let addr = format!("{}:{}", cfg.host, cfg.port).parse().unwrap();
    let mut dev = Device::new(addr, cfg.device_id, &cfg.token, &cfg.key).unwrap();
    let t = Instant::now();
    dev.authenticate().unwrap();
    println!("autenticado em {:?}", t.elapsed());
    let t = Instant::now();
    dev.get_capabilities().unwrap();
    println!("capacidades em {:?}: {:#?}", t.elapsed(), dev.caps);
    let t = Instant::now();
    dev.refresh().unwrap();
    println!("estado em {:?}: {:#?}", t.elapsed(), dev.state);
    if let Some(wait) = std::env::args().nth(1).and_then(|s| s.parse().ok()) {
        for _ in 0..3 {
            std::thread::sleep(Duration::from_secs(wait));
            let t = Instant::now();
            match dev.refresh() {
                Ok(()) => println!("após {wait}s parado: leitura em {:?}, ambiente {:?}", t.elapsed(), dev.state.indoor_temperature),
                Err(e) => println!("após {wait}s parado: erro {e}"),
            }
        }
    }
}
