//! Ícone da bandeja: quadrado arredondado na cor do modo com a temperatura.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

pub fn load_font() -> Option<FontVec> {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    ["segoeuib.ttf", "arialbd.ttf", "segoeui.ttf"]
        .iter()
        .find_map(|f| std::fs::read(format!(r"{dir}\Fonts\{f}")).ok())
        .and_then(|data| FontVec::try_from_vec(data).ok())
}

/// Desenha o ícone em RGBA (não pré-multiplicado), `size`×`size` pixels.
pub fn render(size: u32, color: [u8; 3], text: &str, font: Option<&FontVec>) -> Vec<u8> {
    let s = size as f32;
    let radius = s * 14.0 / 64.0;
    let mut coverage = vec![0f32; (size * size) as usize];
    // Quadrado arredondado com borda suavizada (distância ao canto, 4×4 amostras por pixel)
    for y in 0..size {
        for x in 0..size {
            let mut hits = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = x as f32 + (sx as f32 + 0.5) / 4.0;
                    let py = y as f32 + (sy as f32 + 0.5) / 4.0;
                    let dx = (radius - px).max(px - (s - radius)).max(0.0);
                    let dy = (radius - py).max(py - (s - radius)).max(0.0);
                    if dx * dx + dy * dy <= radius * radius {
                        hits += 1;
                    }
                }
            }
            coverage[(y * size + x) as usize] = hits as f32 / 16.0;
        }
    }
    let mut text_cov = vec![0f32; (size * size) as usize];
    if let Some(font) = font {
        // Fonte menor para três caracteres, maior para um ou dois
        let px = s * if text.chars().count() > 2 { 0.5 } else { 0.68 };
        let scaled = font.as_scaled(PxScale::from(px));
        let width: f32 = text.chars().map(|c| scaled.h_advance(font.glyph_id(c))).sum();
        // Centraliza pela altura das maiúsculas/dígitos, não pela caixa da fonte
        let cap = font.outline(font.glyph_id('0')).map_or(scaled.ascent() * 0.7, |o| {
            // O ab_glyph guarda os limites com o eixo Y invertido
            (o.bounds.max.y - o.bounds.min.y).abs() * scaled.scale().y / font.height_unscaled()
        });
        let baseline = (s + cap) / 2.0;
        let mut pen = (s - width) / 2.0;
        for c in text.chars() {
            let id = font.glyph_id(c);
            let glyph = id.with_scale_and_position(px, ab_glyph::point(pen, baseline));
            pen += scaled.h_advance(id);
            if let Some(outlined) = font.outline_glyph(glyph) {
                let b = outlined.px_bounds();
                outlined.draw(|gx, gy, v| {
                    let x = b.min.x as i32 + gx as i32;
                    let y = b.min.y as i32 + gy as i32;
                    if (0..size as i32).contains(&x) && (0..size as i32).contains(&y) {
                        let i = (y as u32 * size + x as u32) as usize;
                        text_cov[i] = (text_cov[i] + v).min(1.0);
                    }
                });
            }
        }
    }
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for i in 0..(size * size) as usize {
        let t = text_cov[i];
        let mix = |c: u8| (c as f32 * (1.0 - t) + 255.0 * t).round() as u8;
        rgba.extend_from_slice(&[mix(color[0]), mix(color[1]), mix(color[2]), (coverage[i] * 255.0).round() as u8]);
    }
    rgba
}

/// Cria um HICON a partir de RGBA. Quem chama deve liberar com DestroyIcon.
pub fn to_hicon(rgba: &[u8], size: u32) -> HICON {
    unsafe {
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32), // de cima para baixo
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(std::ptr::null_mut(), &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if color.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, rgba.len());
        for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(rgba.as_chunks::<4>().0) {
            *d = [s[2], s[1], s[0], s[3]]; // BGRA
        }
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, std::ptr::null());
        let icon = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let hicon = CreateIconIndirect(&icon);
        DeleteObject(color);
        DeleteObject(mask);
        hicon
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders_rounded_square_with_text() {
        let font = super::load_font();
        let rgba = super::render(32, [21, 101, 192], "24", font.as_ref());
        assert_eq!(rgba.len(), 32 * 32 * 4);
        // Cantos transparentes, centro opaco
        assert_eq!(rgba[3], 0);
        assert_eq!(rgba[(16 * 32 + 16) * 4 + 3], 255);
        // Com a fonte, há pixels brancos de texto
        if font.is_some() {
            assert!(rgba.chunks(4).any(|p| p[0] > 240 && p[1] > 240 && p[2] > 240));
        }
        if let Ok(out) = std::env::var("ICON_DUMP") {
            for (i, (color, text)) in [([21, 101, 192], "24"), ([110, 110, 110], "…"), ([183, 28, 28], "!"), ([46, 125, 50], "18")].iter().enumerate() {
                let img = super::render(32, *color, text, font.as_ref());
                std::fs::write(format!("{out}/icon{i}.rgba"), img).unwrap();
            }
        }
    }
}
