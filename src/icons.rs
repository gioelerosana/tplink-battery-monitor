//! Generazione delle icone usate nel menu della companion app.
//!
//! Le voci di menu DBusMenu accettano un'icona come PNG (`icon-data`): qui
//! disegnamo piccole icone 16x16 (batteria, barra dati, segnale) e le
//! codifichiamo con un encoder PNG minimale, senza dipendenze esterne.
//!
//! Inoltre forniamo la trasformazione ARGB32 usata per "spegnere" l'icona
//! TP-Link incorporata quando il router non e' raggiungibile.

const CYAN: [u8; 4] = [74, 203, 214, 255];
const GREEN: [u8; 4] = [76, 217, 100, 255];
const RED: [u8; 4] = [255, 69, 58, 255];
const BLUE: [u8; 4] = [10, 132, 255, 255];
const TRACK: [u8; 4] = [255, 255, 255, 38];

// ------------------------------------------------------------------- PNG

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    crc ^ 0xFFFF_FFFF
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// Deflate "stored" (nessuna compressione): icone piccole, output compatto
/// a sufficienza e implementazione banale.
fn zlib_store(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 65535 * 5 + 8);
    out.push(0x78);
    out.push(0x01);
    if raw.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    } else {
        let mut offset = 0;
        while offset < raw.len() {
            let n = (raw.len() - offset).min(65535);
            let last = offset + n >= raw.len();
            out.push(u8::from(last));
            out.extend_from_slice(&(n as u16).to_le_bytes());
            out.extend_from_slice(&(!(n as u16)).to_le_bytes());
            out.extend_from_slice(&raw[offset..offset + n]);
            offset += n;
        }
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

/// Codifica un buffer RGBA (8 bit per canale) in PNG.
pub fn encode_png_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0); // filtro "None"
        let start = y * width as usize * 4;
        raw.extend_from_slice(&rgba[start..start + width as usize * 4]);
    }

    let mut out = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bit, RGBA, deflate, no interlace
    write_chunk(&mut out, b"IHDR", &ihdr);
    write_chunk(&mut out, b"IDAT", &zlib_store(&raw));
    write_chunk(&mut out, b"IEND", &[]);
    out
}

// ---------------------------------------------------------------- Canvas

struct Canvas {
    width: i32,
    height: i32,
    pixels: Vec<u8>,
}

impl Canvas {
    fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; (width * height * 4) as usize],
        }
    }

    fn set(&mut self, x: i32, y: i32, colour: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let index = ((y * self.width + x) * 4) as usize;
        self.pixels[index..index + 4].copy_from_slice(&colour);
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, colour: [u8; 4]) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, colour);
            }
        }
    }

    fn to_png(&self) -> Vec<u8> {
        encode_png_rgba(self.width as u32, self.height as u32, &self.pixels)
    }
}

/// Pallino pieno colorato (16x16) usato come indicatore accanto alla barra
/// testuale a tutta larghezza: le voci di menu GNOME non possono colorare
/// il testo.
fn dot_png(colour: [u8; 4]) -> Vec<u8> {
    let mut canvas = Canvas::new(16, 16);
    let center = 7.5_f32;
    let radius = 6.0_f32;
    for y in 0..16 {
        for x in 0..16 {
            let dx = x as f32 + 0.5 - center;
            let dy = y as f32 + 0.5 - center;
            if dx * dx + dy * dy <= radius * radius {
                canvas.set(x, y, colour);
            }
        }
    }
    canvas.to_png()
}

/// Indicatore batteria: verde, rosso sotto il 20%.
pub fn battery_dot_png(level: Option<i64>) -> Vec<u8> {
    let percent = level.unwrap_or(0) as f64;
    let colour = if percent < 20.0 { RED } else { GREEN };
    dot_png(colour)
}

/// Indicatore consumo dati: sempre blu.
pub fn data_dot_png() -> Vec<u8> {
    dot_png(BLUE)
}

/// Quattro tacche di segnale (0-4).
pub fn signal_png(strength: i64) -> Vec<u8> {
    let mut canvas = Canvas::new(16, 16);
    let active = strength.clamp(0, 4) as i32;
    for i in 0..4i32 {
        let height = 5 + i * 3;
        let x = 1 + i * 4;
        let y = 16 - height;
        let colour = if i < active { CYAN } else { TRACK };
        canvas.rect(x, y, 3, height, colour);
    }
    canvas.to_png()
}

/// Riprogettazione di un pixel ARGB32 per l'icona "spenta" (desaturata,
/// piu' scura e semi-trasparente).
pub fn argb_powered_off(data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    for pixel in out.chunks_exact_mut(4) {
        let alpha = pixel[0] as f32;
        let (r, g, b) = (pixel[1] as f32, pixel[2] as f32, pixel[3] as f32);
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        let mix = |c: f32| -> u8 { (c * 0.25 + luma * 0.30) as u8 };
        pixel[1] = mix(r);
        pixel[2] = mix(g);
        pixel[3] = mix(b);
        pixel[0] = (alpha * 0.45) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_png(bytes: &[u8]) {
        assert_eq!(
            &bytes[..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );
        assert_eq!(&bytes[12..16], b"IHDR");
        assert!(bytes.windows(4).any(|w| w == b"IEND"));
    }

    #[test]
    fn icons_are_valid_png() {
        assert_png(&signal_png(3));
        assert_png(&signal_png(0));
        assert_png(&battery_dot_png(Some(37)));
        assert_png(&data_dot_png());
    }

    #[test]
    fn powered_off_dims_pixels() {
        let source = [255u8, 74, 203, 214];
        let dimmed = argb_powered_off(&source);
        assert!(dimmed[0] < source[0]);
        assert!(dimmed[1] <= source[1]);
    }
}
