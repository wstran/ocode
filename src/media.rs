//! Non-text files: classify a path as text, image or other binary, and (for
//! images) re-encode a terminal-sized PNG ready for the kitty graphics
//! protocol that Ghostty speaks.

use std::fs;
use std::io::{Read, Cursor};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::Engine;
use image::ImageFormat;

/// Longest side (px) an image is re-sampled down to before transmission —
/// keeps the payload small with no visible loss at terminal sizes.
const MAX_DIM: u32 = 1000;

/// Bytes sniffed to classify a file (and, for binaries, to hex-preview).
const SNIFF: usize = 8192;

const HEX_PREVIEW: usize = 256;

/// base64 chunk size for the kitty protocol — must be ≤4096 and a multiple of
/// 4 so each chunk stays on a base64 boundary.
const KITTY_CHUNK: usize = 4096;

pub enum Media {
    Image(ImageDoc),

    Binary(BinaryDoc),
}

pub struct ImageDoc {
    pub path: PathBuf,

    pub format: String,

    pub byte_len: u64,

    pub width: u32,

    pub height: u32,

    /// Re-encoded PNG, base64'd straight into the kitty escape.
    png: Vec<u8>,
}

pub struct BinaryDoc {
    pub path: PathBuf,

    pub format: String,

    pub byte_len: u64,

    pub head: Vec<u8>,
}

pub enum Loaded {
    Text,

    Media(Media),
}

/// Decide how to open `path`. Reads only a prefix unless the file turns out to
/// be an image (which must be decoded whole).
pub fn classify(path: &Path) -> Result<Loaded> {
    let meta = fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;

    let byte_len = meta.len();

    let mut prefix = read_prefix(path, SNIFF)?;

    if let Ok(format) = image::guess_format(&prefix) {
        if let Some(doc) = decode_image(path, byte_len, format) {
            return Ok(Loaded::Media(Media::Image(doc)));
        }
    }

    // A known binary magic wins over the UTF-8 sniff: some binaries (e.g. a PDF
    // header) start with plain ASCII yet are not text.
    if magic_format(&prefix).is_none() && looks_textual(&prefix) {
        return Ok(Loaded::Text);
    }

    let format = describe_binary(path, &prefix);

    prefix.truncate(HEX_PREVIEW);

    Ok(Loaded::Media(Media::Binary(BinaryDoc {
        path: path.to_path_buf(),
        format,
        byte_len,
        head: prefix,
    })))
}

fn read_prefix(path: &Path, max: usize) -> Result<Vec<u8>> {
    let mut file = fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;

    let mut buf = vec![0u8; max];

    let mut filled = 0;

    while filled < max {
        let n = file.read(&mut buf[filled..])?;

        if n == 0 {
            break;
        }

        filled += n;
    }

    buf.truncate(filled);

    Ok(buf)
}

/// Valid UTF-8, tolerating a multibyte char clipped by the sniff window.
fn looks_textual(bytes: &[u8]) -> bool {
    match std::str::from_utf8(bytes) {
        Ok(_) => true,

        Err(e) => e.error_len().is_none() && e.valid_up_to() > 0,
    }
}

fn decode_image(path: &Path, byte_len: u64, format: ImageFormat) -> Option<ImageDoc> {
    let bytes = fs::read(path).ok()?;

    let img = image::load_from_memory_with_format(&bytes, format).ok()?;

    let scaled = if img.width().max(img.height()) > MAX_DIM {
        img.resize(MAX_DIM, MAX_DIM, image::imageops::FilterType::Triangle)
    } else {
        img
    };

    let (width, height) = (scaled.width(), scaled.height());

    let mut png = Vec::new();

    scaled.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).ok()?;

    Some(ImageDoc {
        path: path.to_path_buf(),
        format: format_name(format),
        byte_len,
        width,
        height,
        png,
    })
}

fn format_name(format: ImageFormat) -> String {
    match format {
        ImageFormat::Png => "PNG",
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::Gif => "GIF",
        ImageFormat::WebP => "WebP",
        ImageFormat::Bmp => "BMP",
        _ => "image",
    }
    .to_string()
}

/// Identify a file by its leading magic bytes, when we recognise it.
fn magic_format(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"%PDF") {
        Some("PDF document")
    } else if head.starts_with(b"PK\x03\x04") {
        Some("ZIP archive")
    } else if head.starts_with(&[0x1f, 0x8b]) {
        Some("gzip archive")
    } else if head.starts_with(b"\x7fELF") {
        Some("ELF binary")
    } else if head.starts_with(&[0xca, 0xfe, 0xba, 0xbe]) || head.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]) {
        Some("Mach-O binary")
    } else if head.starts_with(b"\0asm") {
        Some("WebAssembly module")
    } else {
        None
    }
}

fn describe_binary(path: &Path, head: &[u8]) -> String {
    if let Some(name) = magic_format(head) {
        return name.to_string();
    }

    let by_ext = match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("mp3" | "wav" | "flac" | "ogg" | "m4a") => "audio file",
        Some("mp4" | "mov" | "mkv" | "webm" | "avi") => "video file",
        Some("ttf" | "otf" | "woff" | "woff2") => "font file",
        Some("zip" | "tar" | "7z" | "rar") => "archive",
        _ => "binary file",
    };

    by_ext.to_string()
}

impl ImageDoc {
    /// The cell box the image actually occupies inside a `cols`×`rows` area,
    /// aspect preserved, so the caller can centre it in the leftover space.
    pub fn fitted_cells(&self, cols: u16, rows: u16) -> (u16, u16) {
        fit_cells(self.width, self.height, cols, rows)
    }

    /// kitty escape(s) to transmit + display the image, scaled to fit a
    /// `cols`×`rows` cell box at the current cursor position (aspect preserved,
    /// cursor left in place so it never scrolls the view).
    pub fn kitty_sequence(&self, cols: u16, rows: u16) -> Vec<u8> {
        let (c, r) = fit_cells(self.width, self.height, cols, rows);

        let b64 = base64::engine::general_purpose::STANDARD.encode(&self.png);

        let bytes = b64.as_bytes();

        let total = bytes.len();

        let mut out = Vec::with_capacity(total + 256);

        let mut i = 0;

        while i < total {
            let end = (i + KITTY_CHUNK).min(total);

            let more = u8::from(end < total);

            out.extend_from_slice(b"\x1b_G");

            if i == 0 {
                let header = format!("a=T,f=100,c={c},r={r},C=1,q=2,m={more};");

                out.extend_from_slice(header.as_bytes());
            } else {
                let header = format!("m={more};");

                out.extend_from_slice(header.as_bytes());
            }

            out.extend_from_slice(&bytes[i..end]);

            out.extend_from_slice(b"\x1b\\");

            i = end;
        }

        out
    }
}

/// Fit `w`×`h` pixels into at most `cols`×`rows` cells, assuming a cell is about
/// twice as tall as it is wide so pixels stay roughly square.
fn fit_cells(w: u32, h: u32, cols: u16, rows: u16) -> (u16, u16) {
    if w == 0 || h == 0 || cols == 0 || rows == 0 {
        return (cols.max(1), rows.max(1));
    }

    let cols_per_row = (w as f64 / h as f64) * 2.0;

    let want_cols = (rows as f64 * cols_per_row).round();

    if want_cols <= cols as f64 {
        ((want_cols.max(1.0)) as u16, rows)
    } else {
        let want_rows = (cols as f64 / cols_per_row).round().clamp(1.0, rows as f64);

        (cols, want_rows as u16)
    }
}

/// Delete every kitty image placement (on leaving an image view or quitting).
pub fn kitty_delete() -> &'static [u8] {
    b"\x1b_Ga=d,q=2\x1b\\"
}

/// Human-readable byte count for the status bar.
pub fn human_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    let mut size = n as f64;

    let mut unit = 0;

    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;

        unit += 1;
    }

    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_text_and_binary() {
        let dir = std::env::temp_dir().join(format!("ocode_media_{}", std::process::id()));

        fs::create_dir_all(&dir).unwrap();

        let txt = dir.join("a.rs");

        fs::write(&txt, "fn main() {}\n").unwrap();

        assert!(matches!(classify(&txt).unwrap(), Loaded::Text));

        let pdf = dir.join("b.pdf");

        fs::write(&pdf, b"%PDF-1.4\n\x00\x01\x02binary").unwrap();

        match classify(&pdf).unwrap() {
            Loaded::Media(Media::Binary(d)) => assert_eq!(d.format, "PDF document"),

            _ => panic!("expected binary"),
        }

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn classifies_png_as_image() {
        let dir = std::env::temp_dir().join(format!("ocode_media_png_{}", std::process::id()));

        fs::create_dir_all(&dir).unwrap();

        let path = dir.join("p.png");

        let mut img = image::RgbaImage::new(20, 10);

        for px in img.pixels_mut() {
            *px = image::Rgba([10, 20, 30, 255]);
        }

        image::DynamicImage::ImageRgba8(img).save(&path).unwrap();

        match classify(&path).unwrap() {
            Loaded::Media(Media::Image(doc)) => {
                assert_eq!(doc.format, "PNG");

                assert_eq!((doc.width, doc.height), (20, 10));

                let seq = doc.kitty_sequence(80, 24);

                assert!(seq.starts_with(b"\x1b_Ga=T,f=100,"), "kitty header missing");

                assert!(seq.windows(2).any(|w| w == b"\x1b\\"), "kitty terminator missing");
            }

            _ => panic!("expected image"),
        }

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fit_cells_is_idempotent() {
        // The placement fits once to centre the image, and kitty_sequence fits
        // again on the way out; the second pass must not shrink it further.
        for (w, h) in [(1000u32, 100u32), (100, 1000), (640, 480), (1, 1), (20, 10)] {
            let (c, r) = fit_cells(w, h, 80, 24);

            assert_eq!(fit_cells(w, h, c, r), (c, r), "for {w}x{h}");
        }
    }

    #[test]
    fn fit_cells_preserves_orientation() {
        // wide image -> limited by columns
        let (c, r) = fit_cells(1000, 100, 80, 24);

        assert!(c <= 80 && r <= 24 && c >= r);

        // tall image -> limited by rows
        let (c, r) = fit_cells(100, 1000, 80, 24);

        assert!(c <= 80 && r <= 24 && r >= c);
    }

    #[test]
    fn human_size_reads_well() {
        assert_eq!(human_size(512), "512 B");

        assert_eq!(human_size(2048), "2.0 KB");
    }
}
