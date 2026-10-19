//! Obtém o token/key V3 do aparelho e monta o config.json (antigo get_token.py).
//!
//! Tenta as nuvens SmartHome e NetHome Plus, com a conta informada (se houver)
//! e com as contas públicas, nos dois formatos de udpid. O token só é aceito
//! depois que o próprio aparelho autentica com ele.

use crate::cloud::{self, Cloud, Kind};
use crate::config::Config;
use crate::discover::Found;
use crate::lan::Lan;

/// Conta Midea do usuário (opcional).
pub struct Account {
    pub email: String,
    pub password: String,
}

pub fn obtain(dev: &Found, account: Option<&Account>, mut report: impl FnMut(String)) -> Result<Config, String> {
    let mut progress = |msg: String| {
        log::info!("Configuração: {msg}");
        report(msg);
    };
    let mut attempts: Vec<(Kind, String, String, String)> = Vec::new();
    if let Some(a) = account {
        for kind in [Kind::SmartHome, Kind::NetHome] {
            attempts.push((kind, "sua conta".into(), a.email.clone(), a.password.clone()));
        }
    }
    for kind in [Kind::SmartHome, Kind::NetHome] {
        for (region, email, password) in kind.builtin_accounts() {
            attempts.push((kind, format!("conta pública {region}"), email.to_string(), password.to_string()));
        }
    }

    for (kind, label, email, password) in attempts {
        let name = format!("{} ({label})", kind.name());
        progress(format!("{name}: entrando…"));
        let mut cloud = Cloud::new(kind, &email, &password);
        if let Err(e) = cloud.login() {
            progress(format!("{name}: login falhou — {e}"));
            continue;
        }
        for big_endian in [false, true] {
            let udpid = cloud::udpid(dev.id, big_endian);
            let (token, key) = match cloud.get_token(&udpid, dev.id) {
                Ok(t) => t,
                Err(e) => {
                    progress(format!("{name}: sem token — {e}"));
                    continue;
                }
            };
            progress(format!("{name}: token recebido, testando no aparelho…"));
            let ok = Lan::new(dev.addr, dev.id, &token, &key).and_then(|mut lan| lan.authenticate());
            match ok {
                Ok(()) => {
                    progress(format!("{name}: o aparelho aceitou a chave"));
                    log::info!("Chave obtida via {name}");
                    return Ok(Config::new(dev.addr.ip().to_string(), dev.addr.port(), dev.id, token, key));
                }
                Err(e) => progress(format!("{name}: o aparelho recusou o token — {e}")),
            }
        }
    }
    Err("Nenhuma combinação de nuvem e conta funcionou.".into())
}
