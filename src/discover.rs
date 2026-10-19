//! Busca de aparelhos Midea na rede local por broadcast UDP (sem nuvem).

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::lan;

#[rustfmt::skip]
const DISCOVERY_MSG: [u8; 72] = [
    0x5a, 0x5a, 0x01, 0x11, 0x48, 0x00, 0x92, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7f, 0x75, 0xbd, 0x6b, 0x3e, 0x4f, 0x8b, 0x76,
    0x2e, 0x84, 0x9c, 0x6e, 0x57, 0x8d, 0x65, 0x90, 0x03, 0x6e, 0x9d, 0x43, 0x42, 0xa5, 0x0f, 0x1f,
    0x56, 0x9e, 0xb8, 0xec, 0x91, 0x8e, 0x92, 0xe5,
];

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub addr: SocketAddr,
    pub id: u64,
    /// Versão do protocolo (2 ou 3; só a 3 precisa de token).
    pub version: u8,
    /// Tipo do aparelho (0xAC = ar-condicionado), tirado do nome "net_ac_XXXX".
    pub device_type: Option<u8>,
}

/// Decodifica uma resposta de descoberta V2/V3. Usa o IP de onde ela veio.
fn parse(ip: Ipv4Addr, data: &[u8]) -> Option<Found> {
    let (data, version) = match data.get(..2)? {
        [0x83, 0x70] => (data.get(8..data.len().checked_sub(16)?)?, 3),
        [0x5A, 0x5A] => (data, 2),
        _ => return None,
    };
    let id_bytes: [u8; 6] = data.get(20..26)?.try_into().ok()?;
    let mut id = [0u8; 8];
    id[..6].copy_from_slice(&id_bytes);
    let decrypted = lan::ecb_decrypt(data.get(40..data.len().checked_sub(16)?)?).ok()?;
    let port = u16::from_le_bytes([*decrypted.get(4)?, *decrypted.get(5)?]);
    let name_len = *decrypted.get(40)? as usize;
    let name = decrypted.get(41..41 + name_len).map(String::from_utf8_lossy).unwrap_or_default();
    let device_type = name.split('_').nth(1).and_then(|t| u8::from_str_radix(t, 16).ok());
    Some(Found { addr: SocketAddr::new(ip.into(), port), id: u64::from_le_bytes(id), version, device_type })
}

pub fn discover(timeout: Duration) -> std::io::Result<Vec<Found>> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    sock.set_broadcast(true)?;
    for port in [6445, 20086] {
        for _ in 0..3 {
            sock.send_to(&DISCOVERY_MSG, (Ipv4Addr::BROADCAST, port))?;
        }
    }
    let deadline = Instant::now() + timeout;
    let mut found: Vec<Found> = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        sock.set_read_timeout(Some(left))?;
        match sock.recv_from(&mut buf) {
            Ok((n, SocketAddr::V4(from))) => {
                if found.iter().any(|f| f.addr.ip() == *from.ip()) {
                    continue;
                }
                match parse(*from.ip(), &buf[..n]) {
                    Some(f) => {
                        log::info!("Encontrado aparelho {} em {}", f.id, f.addr);
                        found.push(f);
                    }
                    None => log::debug!("Resposta de descoberta inválida de {from}"),
                }
            }
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
            // No Windows, um ICMP "porta inacessível" de outro host aparece como erro de recv
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(found)
}
