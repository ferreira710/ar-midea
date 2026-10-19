//! Acesso mínimo às nuvens da Midea, só para obter o token/key V3 do aparelho.
//!
//! Porte de `msmart/cloud.py` (msmart-ng 2026.9.0): SmartHome (MSmartHome) e
//! NetHome Plus, com as contas públicas que a msmart embute.

use std::time::Duration;

use hmac::{Hmac, Mac};
use md5::{Digest as _, Md5};
use serde_json::{json, Map, Value};
use sha2::Sha256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    SmartHome,
    NetHome,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::SmartHome => "SmartHome",
            Kind::NetHome => "NetHome Plus",
        }
    }

    fn app_id(self) -> &'static str {
        match self {
            Kind::SmartHome => "1010",
            Kind::NetHome => "1017",
        }
    }

    /// Contas públicas por região, na ordem da msmart.
    pub fn builtin_accounts(self) -> &'static [(&'static str, &'static str, &'static str)] {
        match self {
            Kind::SmartHome => &[
                ("DE", "midea_eu@mailinator.com", "das_ist_passwort1"),
                ("KR", "midea_sea@mailinator.com", "password_for_sea1"),
                ("US", "midea@mailinator.com", "this_is_a_password1"),
            ],
            Kind::NetHome => &[
                ("DE", "nethome+de@mailinator.com", "password1"),
                ("KR", "nethome+sea@mailinator.com", "password1"),
                ("US", "nethome+us@mailinator.com", "password1"),
            ],
        }
    }
}

const SMARTHOME_URL: &str = "https://mp-prod.appsmb.com";
const SMARTHOME_HMAC_KEY: &str = "PROD_VnoClJI9aikS8dyy";
const SMARTHOME_IOT_KEY: &str = "meicloud";
const SMARTHOME_LOGIN_KEY: &str = "ac21b9f9cbfe4ca5a88562ef25e2b768";
const NETHOME_URL: &str = "https://mapp.appsmb.com";
const NETHOME_APP_KEY: &str = "3742e9e5842d4ad59c2db887e12449f9";

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).expect("gerador aleatório do sistema");
    hex::encode(buf)
}

fn random_urlsafe(bytes: usize) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).expect("gerador aleatório do sistema");
    let mut out = String::new();
    for chunk in buf.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 0x3F) as usize] as char);
        }
    }
    out
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn md5_hex(s: &str) -> String {
    hex::encode(Md5::digest(s.as_bytes()))
}

fn stamp() -> String {
    let t = crate::lan::utc_now();
    format!("{:04}{:02}{:02}{:02}{:02}{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second)
}

/// ID de 6 bytes do aparelho → "udpid" que a nuvem usa para achar o token.
pub fn udpid(device_id: u64, big_endian: bool) -> String {
    let le = device_id.to_le_bytes();
    let mut id: [u8; 6] = le[..6].try_into().unwrap();
    if big_endian {
        id.reverse();
    }
    let hash = Sha256::digest(id);
    hex::encode((0..16).map(|i| hash[i] ^ hash[i + 16]).collect::<Vec<u8>>())
}

pub struct Cloud {
    kind: Kind,
    account: String,
    password: String,
    agent: ureq::Agent,
    device_id: String,
    access_token: String,
    session_id: String,
}

impl Cloud {
    pub fn new(kind: Kind, account: &str, password: &str) -> Cloud {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .http_status_as_error(false)
            .build();
        Cloud {
            kind,
            account: account.into(),
            password: password.into(),
            agent: config.into(),
            device_id: random_hex(8),
            access_token: String::new(),
            session_id: String::new(),
        }
    }

    fn base_body(&self) -> Map<String, Value> {
        let mut body = Map::new();
        body.insert("appId".into(), json!(self.kind.app_id()));
        body.insert("src".into(), json!(self.kind.app_id()));
        body.insert("format".into(), json!(2));
        body.insert("clientType".into(), json!(1));
        body.insert("language".into(), json!("en_US"));
        body.insert("deviceId".into(), json!(self.device_id));
        body.insert("stamp".into(), json!(stamp()));
        match self.kind {
            Kind::SmartHome => body.insert("reqId".into(), json!(random_hex(16))),
            Kind::NetHome => body.insert("sessionId".into(), json!(self.session_id)),
        };
        body
    }

    fn request(&self, endpoint: &str, body: Map<String, Value>) -> Result<Value, String> {
        let text = match self.kind {
            Kind::SmartHome => {
                let contents = Value::Object(body).to_string();
                let random = random_hex(16);
                let mut mac = Hmac::<Sha256>::new_from_slice(SMARTHOME_HMAC_KEY.as_bytes()).unwrap();
                mac.update(format!("{SMARTHOME_IOT_KEY}{contents}{random}").as_bytes());
                let sign = hex::encode(mac.finalize().into_bytes());
                self.agent
                    .post(format!("{SMARTHOME_URL}/mas/v5/app/proxy?alias={endpoint}"))
                    .header("Content-Type", "application/json")
                    .header("secretVersion", "1")
                    .header("sign", sign)
                    .header("random", random)
                    .header("accessToken", &self.access_token)
                    .send(contents)
            }
            Kind::NetHome => {
                // Assinatura: caminho + campos ordenados "k=v&..." + chave do app
                let mut fields: Vec<(String, String)> = body
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().map_or_else(|| v.to_string(), String::from)))
                    .collect();
                fields.sort();
                let query = fields.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
                fields.push(("sign".into(), sha256_hex(&format!("{endpoint}{query}{NETHOME_APP_KEY}"))));
                self.agent.post(format!("{NETHOME_URL}{endpoint}")).send_form(fields)
            }
        }
        .map_err(|e| format!("falha na requisição: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("falha ao ler a resposta: {e}"))?;

        let reply: Value = serde_json::from_str(&text).map_err(|_| format!("resposta inesperada: {text}"))?;
        let (code_field, data_field) = match self.kind {
            Kind::SmartHome => ("code", "data"),
            Kind::NetHome => ("errorCode", "result"),
        };
        let code = match &reply[code_field] {
            Value::String(s) => s.parse().unwrap_or(-1),
            v => v.as_i64().unwrap_or(-1),
        };
        if code != 0 {
            let msg = reply["msg"].as_str().unwrap_or("erro desconhecido").replace('，', ", ");
            return Err(format!("código {code}: {msg}"));
        }
        Ok(reply[data_field].clone())
    }

    pub fn login(&mut self) -> Result<(), String> {
        let mut body = self.base_body();
        body.insert("loginAccount".into(), json!(self.account));
        let login_id = self.request("/v1/user/login/id/get", body)?["loginId"]
            .as_str()
            .ok_or("resposta sem loginId")?
            .to_string();

        match self.kind {
            Kind::SmartHome => {
                let password = sha256_hex(&format!("{login_id}{}{SMARTHOME_LOGIN_KEY}", sha256_hex(&self.password)));
                let iam = sha256_hex(&format!("{login_id}{}{SMARTHOME_LOGIN_KEY}", md5_hex(&md5_hex(&self.password))));
                let body = json!({
                    "data": { "platform": 2, "deviceId": self.device_id },
                    "iotData": {
                        "appId": "1010",
                        "src": "1010",
                        "clientType": 1,
                        "loginAccount": self.account,
                        "iampwd": iam,
                        "password": password,
                        "pushToken": random_urlsafe(120),
                        "stamp": stamp(),
                        "reqId": random_hex(16),
                    },
                });
                let Value::Object(body) = body else { unreachable!() };
                let reply = self.request("/mj/user/login", body)?;
                self.access_token = reply["mdata"]["accessToken"].as_str().ok_or("resposta sem accessToken")?.into();
            }
            Kind::NetHome => {
                let password = sha256_hex(&format!("{login_id}{}{NETHOME_APP_KEY}", sha256_hex(&self.password)));
                let mut body = self.base_body();
                body.insert("loginAccount".into(), json!(self.account));
                body.insert("password".into(), json!(password));
                let reply = self.request("/v1/user/login", body)?;
                self.session_id = reply["sessionId"].as_str().ok_or("resposta sem sessionId")?.into();
            }
        }
        Ok(())
    }

    /// Devolve (token, key) do aparelho.
    pub fn get_token(&self, udpid: &str, device_id: u64) -> Result<(String, String), String> {
        let mut body = self.base_body();
        body.insert("udpid".into(), json!(udpid));
        // A SmartHome recusa o pedido sem o ID do aparelho; a NetHome ignora
        body.insert("applianceCodes".into(), json!(device_id.to_string()));
        let reply = self.request("/v1/iot/secure/getToken", body)?;
        reply["tokenlist"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| t["udpId"].as_str() == Some(udpid))
            .and_then(|t| Some((t["token"].as_str()?.to_string(), t["key"].as_str()?.to_string())))
            .ok_or_else(|| format!("nenhum token para o udpid {udpid}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udpid_matches_msmart() {
        // Mesmo cálculo da msmart: sha256 do ID em 6 bytes, metades combinadas por XOR
        let id: u64 = 150633093310969;
        let le = Sha256::digest(&id.to_le_bytes()[..6]);
        let expected: Vec<u8> = (0..16).map(|i| le[i] ^ le[i + 16]).collect();
        assert_eq!(udpid(id, false), hex::encode(expected));
        assert_ne!(udpid(id, false), udpid(id, true));
        assert_eq!(udpid(id, false).len(), 32);
    }

    #[test]
    fn urlsafe_has_expected_length() {
        // token_urlsafe(120) do Python gera 160 caracteres
        assert_eq!(random_urlsafe(120).len(), 160);
        assert!(random_urlsafe(30).chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }
}
