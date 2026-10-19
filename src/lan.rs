//! Protocolo LAN dos aparelhos Midea (pacotes V2 dentro do transporte V3 "8370").
//!
//! Porte da parte que este aparelho usa de `msmart/lan.py` (msmart-ng 2026.9.0).

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::{Aes128, Aes256, Block};
use md5::{Digest as _, Md5};
use sha2::Sha256;

const SIGN_KEY: &[u8] = b"xhdiwjnchekd4d512chdjx5d8e4c394D2D7S";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const RETRIES: usize = 3;
const AUTH_LIFETIME: Duration = Duration::from_secs(12 * 3600);

#[derive(Debug)]
pub enum Error {
    Timeout(String),
    Protocol(String),
    Auth(String),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Timeout(m) | Error::Protocol(m) => f.write_str(m),
            Error::Auth(m) => write!(f, "falha na autenticação: {m}"),
            Error::Io(e) => write!(f, "erro de rede: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Error::Timeout("tempo esgotado".into()),
            _ => Error::Io(e),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

// ---------- criptografia ----------

fn enc_key() -> [u8; 16] {
    Md5::digest(SIGN_KEY).into()
}

fn sign(data: &[u8]) -> [u8; 16] {
    let mut h = Md5::new();
    h.update(data);
    h.update(SIGN_KEY);
    h.finalize().into()
}

fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// AES-256-CBC com IV zerado, sem padding (o chamador alinha em 16 bytes).
fn cbc_encrypt(key: &[u8; 32], data: &[u8]) -> Vec<u8> {
    let cipher = Aes256::new(key.into());
    let mut prev = [0u8; 16];
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.as_chunks::<16>().0 {
        let mut block = Block::default();
        for (b, (c, p)) in block.iter_mut().zip(chunk.iter().zip(&prev)) {
            *b = c ^ p;
        }
        cipher.encrypt_block(&mut block);
        prev.copy_from_slice(&block);
        out.extend_from_slice(&block);
    }
    out
}

fn cbc_decrypt(key: &[u8; 32], data: &[u8]) -> Vec<u8> {
    let cipher = Aes256::new(key.into());
    let mut prev = [0u8; 16];
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.as_chunks::<16>().0 {
        let mut block = Block::from(*chunk);
        cipher.decrypt_block(&mut block);
        out.extend(block.iter().zip(&prev).map(|(b, p)| b ^ p));
        prev = *chunk;
    }
    out
}

/// AES-128-ECB com a chave fixa e padding PKCS#7.
pub(crate) fn ecb_encrypt(data: &[u8]) -> Vec<u8> {
    let cipher = Aes128::new(&enc_key().into());
    let pad = 16 - data.len() % 16;
    let mut buf = data.to_vec();
    buf.extend(std::iter::repeat_n(pad as u8, pad));
    for chunk in buf.as_chunks_mut::<16>().0 {
        cipher.encrypt_block(chunk.into());
    }
    buf
}

pub(crate) fn ecb_decrypt(data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() || !data.len().is_multiple_of(16) {
        return Err(Error::Protocol("dados criptografados com tamanho inválido".into()));
    }
    let cipher = Aes128::new(&enc_key().into());
    let mut buf = data.to_vec();
    for chunk in buf.as_chunks_mut::<16>().0 {
        cipher.decrypt_block(chunk.into());
    }
    let pad = *buf.last().unwrap() as usize;
    if pad == 0 || pad > 16 || buf[buf.len() - pad..].iter().any(|&b| b as usize != pad) {
        return Err(Error::Protocol("padding inválido".into()));
    }
    buf.truncate(buf.len() - pad);
    Ok(buf)
}

// ---------- pacote V2 (5A5A) ----------

pub(crate) struct UtcTime {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millis: u32,
}

pub(crate) fn utc_now() -> UtcTime {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs();
    // Conversão de dias desde 1970 para data civil (algoritmo de Howard Hinnant)
    let z = (secs / 86400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    UtcTime {
        year: year as u32,
        month: month as u32,
        day: day as u32,
        hour: ((secs / 3600) % 24) as u32,
        minute: ((secs / 60) % 60) as u32,
        second: (secs % 60) as u32,
        millis: now.subsec_millis(),
    }
}

/// Data/hora UTC no formato do pacote: centésimos, s, min, h, dia, mês, ano%100, ano/100.
fn timestamp() -> [u8; 8] {
    let t = utc_now();
    [
        (t.millis / 10) as u8,
        t.second as u8,
        t.minute as u8,
        t.hour as u8,
        t.day as u8,
        t.month as u8,
        (t.year % 100) as u8,
        (t.year / 100) as u8,
    ]
}

pub(crate) fn encode_v2(device_id: u64, frame: &[u8]) -> Vec<u8> {
    let encrypted = ecb_encrypt(frame);
    let length = (40 + encrypted.len() + 16) as u16;
    let mut packet = Vec::with_capacity(length as usize);
    packet.extend_from_slice(&[0x5A, 0x5A, 0x01, 0x11]);
    packet.extend_from_slice(&length.to_le_bytes());
    packet.extend_from_slice(&[0x20, 0x00]);
    packet.extend_from_slice(&[0; 4]); // ID da mensagem
    packet.extend_from_slice(&timestamp());
    packet.extend_from_slice(&device_id.to_le_bytes());
    packet.extend_from_slice(&[0; 12]);
    packet.extend_from_slice(&encrypted);
    let signature = sign(&packet);
    packet.extend_from_slice(&signature);
    packet
}

pub(crate) fn decode_v2(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 6 || data[..2] != [0x5A, 0x5A] {
        return Err(Error::Protocol(format!("pacote não suportado: {}", hex::encode(data))));
    }
    let length = u16::from_le_bytes([data[4], data[5]]) as usize;
    if length < 56 || data.len() < length {
        return Err(Error::Protocol("pacote truncado".into()));
    }
    let packet = &data[..length];
    if sign(&packet[..length - 16]) != packet[length - 16..] {
        return Err(Error::Protocol("assinatura MD5 não confere".into()));
    }
    ecb_decrypt(&packet[40..length - 16])
}

// ---------- transporte V3 (8370) ----------

const TYPE_HANDSHAKE_REQUEST: u8 = 0x0;
const TYPE_HANDSHAKE_RESPONSE: u8 = 0x1;
const TYPE_ENCRYPTED_RESPONSE: u8 = 0x3;
const TYPE_ENCRYPTED_REQUEST: u8 = 0x6;
const TYPE_ERROR: u8 = 0xF;

struct Conn {
    stream: TcpStream,
    buf: Vec<u8>,
    packet_id: u16,
    local_key: Option<[u8; 32]>,
    expires: Instant,
}

impl Conn {
    fn connect(addr: SocketAddr) -> Result<Conn> {
        log::info!("Conectando a {addr}");
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| match e.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
                Error::Timeout(format!("tempo esgotado ao conectar a {addr}"))
            }
            _ => Error::Protocol(format!("não foi possível conectar a {addr}: {e}")),
        })?;
        stream.set_nodelay(true).ok();
        Ok(Conn { stream, buf: Vec::new(), packet_id: 0, local_key: None, expires: Instant::now() })
    }

    fn authenticated(&self) -> bool {
        self.local_key.is_some() && Instant::now() < self.expires
    }

    fn write(&mut self, data: &[u8], packet_type: u8) -> Result<()> {
        let mut packet = Vec::with_capacity(data.len() + 64);
        packet.extend_from_slice(&[0x83, 0x70]);
        if packet_type == TYPE_HANDSHAKE_REQUEST {
            packet.extend_from_slice(&(data.len() as u16).to_be_bytes());
            packet.extend_from_slice(&[0x20, TYPE_HANDSHAKE_REQUEST]);
            packet.extend_from_slice(&self.packet_id.to_be_bytes());
            packet.extend_from_slice(data);
        } else {
            let key = self.local_key.ok_or_else(|| Error::Protocol("conexão não autenticada".into()))?;
            let remainder = (data.len() + 2) % 16;
            let pad = if remainder == 0 { 0 } else { 16 - remainder };
            packet.extend_from_slice(&((data.len() + pad + 32) as u16).to_be_bytes());
            packet.extend_from_slice(&[0x20, (pad as u8) << 4 | TYPE_ENCRYPTED_REQUEST]);
            let mut payload = Vec::with_capacity(data.len() + 2 + pad);
            payload.extend_from_slice(&self.packet_id.to_be_bytes());
            payload.extend_from_slice(data);
            let mut padding = [0u8; 16];
            getrandom::getrandom(&mut padding[..pad]).map_err(|e| Error::Protocol(e.to_string()))?;
            payload.extend_from_slice(&padding[..pad]);
            let hash = sha256(&[&packet, &payload]);
            packet.extend_from_slice(&cbc_encrypt(&key, &payload));
            packet.extend_from_slice(&hash);
        }
        self.stream.write_all(&packet)?;
        self.packet_id = (self.packet_id + 1) & 0xFFF;
        Ok(())
    }

    /// Tira do buffer um pacote 8370 completo, se houver.
    fn take_packet(&mut self) -> Option<Vec<u8>> {
        let Some(start) = self.buf.windows(2).position(|w| w == [0x83, 0x70]) else {
            // Guarda um 0x83 final, que pode ser o começo de um pacote ainda chegando
            let keep = usize::from(self.buf.last() == Some(&0x83));
            if self.buf.len() > keep {
                log::warn!("Descartando dados sem início de pacote: {}", hex::encode(&self.buf));
                self.buf.drain(..self.buf.len() - keep);
            }
            return None;
        };
        if start > 0 {
            log::warn!("Ignorando dados antes do pacote: {}", hex::encode(&self.buf[..start]));
            self.buf.drain(..start);
        }
        if self.buf.len() < 6 {
            return None;
        }
        let total = u16::from_be_bytes([self.buf[2], self.buf[3]]) as usize + 8;
        if self.buf.len() < total {
            return None;
        }
        Some(self.buf.drain(..total).collect())
    }

    /// Lê um pacote 8370 completo. Com `timeout` zero, só devolve o que já chegou.
    fn read_packet(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let deadline = Instant::now() + timeout;
        let mut chunk = [0u8; 4096];
        loop {
            if let Some(p) = self.take_packet() {
                return Ok(Some(p));
            }
            let n = if timeout.is_zero() {
                self.stream.set_nonblocking(true)?;
                let r = self.stream.read(&mut chunk);
                self.stream.set_nonblocking(false)?;
                match r {
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                    r => r?,
                }
            } else {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(Error::Timeout("sem resposta do aparelho".into()));
                }
                self.stream.set_read_timeout(Some(left))?;
                match self.stream.read(&mut chunk) {
                    Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => {
                        return Err(Error::Timeout("sem resposta do aparelho".into()));
                    }
                    r => r?,
                }
            };
            if n == 0 {
                return Err(Error::Io(io::Error::new(io::ErrorKind::ConnectionAborted, "conexão fechada pelo aparelho")));
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    fn process(&self, packet: &[u8]) -> Result<Vec<u8>> {
        if packet[4] != 0x20 {
            return Err(Error::Protocol(format!("byte mágico inválido: 0x{:X}", packet[4])));
        }
        match packet[5] & 0xF {
            TYPE_ENCRYPTED_RESPONSE => {
                let key = self.local_key.ok_or_else(|| Error::Protocol("resposta criptografada antes da autenticação".into()))?;
                if packet.len() < 6 + 32 + 16 || !(packet.len() - 38).is_multiple_of(16) {
                    return Err(Error::Protocol("pacote criptografado com tamanho inválido".into()));
                }
                let (header, rest) = packet.split_at(6);
                let (payload, hash) = rest.split_at(rest.len() - 32);
                let decrypted = cbc_decrypt(&key, payload);
                if sha256(&[header, &decrypted]) != hash {
                    return Err(Error::Protocol("hash SHA256 não confere".into()));
                }
                let pad = (header[5] >> 4) as usize;
                if decrypted.len() < 2 + pad {
                    return Err(Error::Protocol("pacote criptografado curto demais".into()));
                }
                Ok(decrypted[2..decrypted.len() - pad].to_vec())
            }
            TYPE_HANDSHAKE_RESPONSE => Ok(packet.get(8..).unwrap_or_default().to_vec()),
            TYPE_ERROR => Err(Error::Protocol("o aparelho respondeu com erro".into())),
            t => Err(Error::Protocol(format!("tipo de pacote inesperado: {t}"))),
        }
    }

    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        match self.read_packet(timeout)? {
            Some(p) => self.process(&p).map(Some),
            None => Ok(None),
        }
    }

    fn authenticate(&mut self, token: &[u8], key: &[u8; 32]) -> Result<()> {
        self.buf.clear();
        let mut attempt = 0;
        let response = loop {
            attempt += 1;
            self.write(token, TYPE_HANDSHAKE_REQUEST).map_err(|e| Error::Auth(e.to_string()))?;
            match self.read(READ_TIMEOUT) {
                Ok(Some(r)) => break r,
                Err(Error::Timeout(_)) if attempt < RETRIES => continue,
                Err(Error::Timeout(_)) => return Err(Error::Timeout("sem resposta do aparelho".into())),
                Err(e) => return Err(Error::Auth(e.to_string())),
                Ok(None) => unreachable!(),
            }
        };
        if response.len() != 64 {
            return Err(Error::Auth("resposta do handshake com tamanho inválido".into()));
        }
        let decrypted = cbc_decrypt(key, &response[..32]);
        if sha256(&[&decrypted]) != response[32..] {
            return Err(Error::Auth("hash do handshake não confere (token/key errados?)".into()));
        }
        let mut local = [0u8; 32];
        for i in 0..32 {
            local[i] = decrypted[i] ^ key[i];
        }
        self.local_key = Some(local);
        self.expires = Instant::now() + AUTH_LIFETIME;
        Ok(())
    }
}

/// Conexão com um aparelho V3, reconectando e reautenticando quando preciso.
pub struct Lan {
    addr: SocketAddr,
    device_id: u64,
    token: Vec<u8>,
    key: [u8; 32],
    conn: Option<Conn>,
}

impl Lan {
    pub fn new(addr: SocketAddr, device_id: u64, token: &str, key: &str) -> Result<Lan> {
        let token = hex::decode(token).map_err(|_| Error::Auth("token inválido no config.json".into()))?;
        let key: [u8; 32] = hex::decode(key)
            .ok()
            .and_then(|k| k.try_into().ok())
            .ok_or_else(|| Error::Auth("key inválida no config.json".into()))?;
        Ok(Lan { addr, device_id, token, key, conn: None })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn set_addr(&mut self, addr: SocketAddr) {
        self.addr = addr;
        self.conn = None;
    }

    pub fn disconnect(&mut self) {
        self.conn = None;
    }

    /// Abre uma conexão nova e autentica.
    pub fn authenticate(&mut self) -> Result<()> {
        self.conn = None;
        let mut conn = Conn::connect(self.addr)?;
        log::info!("Autenticando com {}", self.addr);
        conn.authenticate(&self.token, &self.key)?;
        log::info!("Autenticado com {}", self.addr);
        self.conn = Some(conn);
        // O aparelho ignora pedidos feitos logo após o handshake
        std::thread::sleep(Duration::from_millis(1000));
        Ok(())
    }

    /// Envia um frame e devolve os frames recebidos em resposta.
    pub fn send(&mut self, frame: &[u8]) -> Result<Vec<Vec<u8>>> {
        let fresh = !matches!(&self.conn, Some(c) if c.authenticated());
        if fresh {
            self.authenticate()?;
        }
        match self.send_once(frame) {
            Err(Error::Io(e)) if !fresh => {
                // O aparelho fecha conexões paradas; reconecta uma vez
                log::info!("Conexão perdida ({e}); reconectando");
                self.authenticate()?;
                self.send_once(frame)
            }
            r => r,
        }
    }

    fn send_once(&mut self, frame: &[u8]) -> Result<Vec<Vec<u8>>> {
        let packet = encode_v2(self.device_id, frame);
        let conn = self.conn.as_mut().expect("conexão autenticada");
        let mut responses = Vec::new();
        let result = (|| {
            // Respostas avulsas que chegaram antes do pedido
            while let Some(p) = conn.read(Duration::ZERO)? {
                responses.push(decode_v2(&p)?);
            }
            let mut attempt = 0;
            loop {
                attempt += 1;
                conn.write(&packet, TYPE_ENCRYPTED_REQUEST)?;
                match conn.read(READ_TIMEOUT) {
                    Ok(Some(p)) => {
                        responses.push(decode_v2(&p)?);
                        break;
                    }
                    Err(Error::Timeout(_)) if attempt < RETRIES => {
                        log::debug!("Sem resposta; reenviando");
                    }
                    Err(e) => return Err(e),
                    Ok(None) => unreachable!(),
                }
            }
            while let Some(p) = conn.read(Duration::ZERO)? {
                responses.push(decode_v2(&p)?);
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(responses),
            Err(e) => {
                self.conn = None;
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pacotes reais do aparelho (camada V2, já fora do transporte V3)
    const RX_CAPS: &str = "5a5a011178002080000000004b103114130a1a14f9a9040000890000000000000000018000000000d50e76f9d56b1b007804ff46779892e4b6def5509f29d52a20f3508cef831b26b80e5bf3567f3e76197a3ce12165c38f2aadc35e5d24a8a6946124e5b95985efed6df80d9d6d0c03957413f70c2f273d";
    const RX_STATE: &str = "5a5a01116800208000000000d2503114130a1a14f9a90400008900000000000000000180000000007d9f2738b80c3240c89cea0cb3609bc9e0c5dd5ae0060e322b19e05de9548b74335f8cdfbc9863b3e25281d1ebbc8064ae1badcf523d87cd4419c97f8a7c358d";
    const TX_CAPS: &str = "5a5a0111580020000000000011063114130a1a14f9a90400008900000000000000000000000000000542065231b95b9123b7f675b1de90ae915c618b02ed2747fad73f2ffa0db1d3db6c91f97370bf534f4d1e697441d3e9";

    #[test]
    fn decodes_device_packets() {
        let frame = decode_v2(&hex::decode(RX_CAPS).unwrap()).unwrap();
        assert_eq!(
            hex::encode(frame),
            "aa31ac00000000000203b507120201011402010015020101160201001a02010110020101250207203c203c203c000100bfcf"
        );
        let frame = decode_v2(&hex::decode(RX_STATE).unwrap()).unwrap();
        assert_eq!(hex::encode(frame), "aa28ac00000000000203c00028657f7f003f00000067ff0c0009000000000000000100000000037da1");
    }

    #[test]
    fn encodes_like_msmart() {
        let frame = hex::decode("aa0fac00000000000003b5010001e5a6").unwrap();
        let ours = encode_v2(150633093310969, &frame);
        let theirs = hex::decode(TX_CAPS).unwrap();
        // Tudo igual menos a data/hora (bytes 12..20) e a assinatura
        assert_eq!(ours.len(), theirs.len());
        assert_eq!(ours[..12], theirs[..12]);
        assert_eq!(ours[20..ours.len() - 16], theirs[20..theirs.len() - 16]);
        assert_eq!(decode_v2(&ours).unwrap(), frame);
    }

    #[test]
    fn cbc_roundtrip() {
        let key = [7u8; 32];
        let data: Vec<u8> = (0..48).collect();
        assert_eq!(cbc_decrypt(&key, &cbc_encrypt(&key, &data)), data);
    }

    #[test]
    fn timestamp_is_sane() {
        let t = timestamp();
        assert!(t[0] < 100 && t[1] < 60 && t[2] < 60 && t[3] < 24);
        assert!((1..=31).contains(&t[4]) && (1..=12).contains(&t[5]) && t[7] == 20);
    }
}
