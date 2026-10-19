//! Teste do fluxo de configuração: descobre o aparelho, obtém token/key pela
//! nuvem e valida no aparelho. Não grava nada; compara com o config.json atual.
use std::time::{Duration, Instant};

use ar_midea::config::Config;
use ar_midea::{discover, setup};

fn main() {
    let t = Instant::now();
    let found = discover::discover(Duration::from_secs(3)).expect("descoberta");
    println!("descoberta em {:?}: {:?}", t.elapsed(), found);
    let dev = found.iter().find(|d| d.device_type == Some(0xAC)).expect("nenhum ar-condicionado na rede");
    let t = Instant::now();
    let cfg = setup::obtain(dev, None, |m| println!("  {m}")).expect("obter chave");
    println!("chave obtida em {:?}", t.elapsed());
    match Config::load() {
        Ok(old) => println!(
            "comparado ao config.json atual: token {} | key {} | host {}",
            if old.token.eq_ignore_ascii_case(&cfg.token) { "igual" } else { "diferente" },
            if old.key.eq_ignore_ascii_case(&cfg.key) { "igual" } else { "diferente" },
            if old.host == cfg.host { "igual" } else { "diferente" },
        ),
        Err(e) => println!("sem config.json para comparar: {e}"),
    }
}
