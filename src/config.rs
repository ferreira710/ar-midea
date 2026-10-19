//! config.json gerado pelo get_token.py, na mesma pasta do executável.

use std::path::PathBuf;

use serde_json::{json, Map, Value};

pub fn base_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn path() -> PathBuf {
    base_dir().join("config.json")
}

#[derive(Clone, Debug)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub device_id: u64,
    pub token: String,
    pub key: String,
    /// Campos originais, para regravar o arquivo sem perder nada.
    raw: Map<String, Value>,
}

impl Config {
    pub fn load() -> Result<Config, String> {
        let text = std::fs::read_to_string(path()).map_err(|e| e.to_string())?;
        let raw: Map<String, Value> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let field = |k: &str| raw.get(k).ok_or_else(|| format!("campo \"{k}\" ausente"));
        let text_field = |k: &str| field(k)?.as_str().map(String::from).ok_or_else(|| format!("campo \"{k}\" inválido"));
        Ok(Config {
            host: text_field("host")?,
            port: field("port")?.as_u64().and_then(|p| u16::try_from(p).ok()).ok_or("campo \"port\" inválido")?,
            device_id: field("device_id")?.as_u64().ok_or("campo \"device_id\" inválido")?,
            token: text_field("token")?,
            key: text_field("key")?,
            raw,
        })
    }

    /// Configuração nova (mesma ordem de campos do antigo get_token.py).
    pub fn new(host: String, port: u16, device_id: u64, token: String, key: String) -> Config {
        let mut raw = Map::new();
        raw.insert("host".into(), json!(host));
        raw.insert("port".into(), json!(port));
        raw.insert("device_id".into(), json!(device_id));
        raw.insert("token".into(), json!(token));
        raw.insert("key".into(), json!(key));
        Config { host, port, device_id, token, key, raw }
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        self.raw.insert("host".into(), json!(self.host));
        self.raw.insert("port".into(), json!(self.port));
        self.raw.insert("token".into(), json!(self.token));
        self.raw.insert("key".into(), json!(self.key));
        let text = serde_json::to_string_pretty(&self.raw).map_err(std::io::Error::other)?;
        std::fs::write(path(), text + "\n")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn keeps_field_order() {
        let cfg = super::Config::new("192.168.1.2".into(), 6444, 1, "aa".into(), "bb".into());
        let text = serde_json::to_string(&cfg.raw).unwrap();
        assert_eq!(text, r#"{"host":"192.168.1.2","port":6444,"device_id":1,"token":"aa","key":"bb"}"#);
    }
}
