//! Photos for the Wall kit (pa14): the type from the file's first bytes, and the strip that runs on
//! arrival, before anything is written (plan section 8 rule 12). The strip is a hand-written walk
//! over the file's markers or chunks: segments that carry location, camera or other text are left
//! out and every other byte is copied as it came. Nothing is decoded and no pixel is re-encoded.
//!
//! - JPEG keeps APP0 `JFIF`, APP2 `ICC_PROFILE` and the image segments; it drops APP1 (Exif, XMP),
//!   APP13 (IPTC), COM, every other APPn and anything after the end marker (phones put extra
//!   pictures there, each with its own Exif).
//! - PNG keeps only the chunks in [`PNG_KEEP`] (the image, its colour and APNG frames) and drops
//!   every other one (`eXIf`, text, `tIME`, C2PA `caBX`, any private chunk) and anything after
//!   `IEND`.
//! - WebP keeps only the chunks in [`WEBP_KEEP`] (inside an animation frame only its image and
//!   alpha), sets the `VP8X` flags to match what it kept and fixes the RIFF size.
//!
//! Both are allowlists (pa14 QA F1): a chunk nobody listed, such as C2PA or a maker's own, never
//! gets through. ICC colour profiles stay on purpose: they say how the colours look, not where or
//! with what the photo was taken.
//!
//! The Exif orientation is read before its segment goes and comes back as `rotate` (0, 90, 180 or
//! 270 degrees clockwise), so viewers turn the photo the way the camera held it. The zip alone
//! writes it back into JPEGs ([`jpeg_with_orientation`]), since photo apps don't know `rotate`.

/// A photo type the wall takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
    Webp,
}

impl Kind {
    pub fn ext(self) -> &'static str {
        match self {
            Kind::Jpeg => "jpg",
            Kind::Png => "png",
            Kind::Webp => "webp",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Kind::Jpeg => "image/jpeg",
            Kind::Png => "image/png",
            Kind::Webp => "image/webp",
        }
    }

    pub fn from_ext(ext: &str) -> Option<Kind> {
        match ext {
            "jpg" => Some(Kind::Jpeg),
            "png" => Some(Kind::Png),
            "webp" => Some(Kind::Webp),
            _ => None,
        }
    }
}

/// What the first bytes say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniff {
    Photo(Kind),
    /// HEIC, HEIF or AVIF: a photo, but not one the wall shows (the page converts these first).
    Heic,
    Other,
}

/// The type from magic bytes only: a file name or a Content-Type is never trusted.
pub fn sniff(b: &[u8]) -> Sniff {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Sniff::Photo(Kind::Jpeg);
    }
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Sniff::Photo(Kind::Png);
    }
    if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Sniff::Photo(Kind::Webp);
    }
    if b.len() >= 12 && &b[4..8] == b"ftyp" {
        let brand = &b[8..12];
        if [b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"mif1", b"msf1", b"avif", b"avis"].iter().any(|x| brand == &x[..]) {
            return Sniff::Heic;
        }
    }
    Sniff::Other
}

/// A photo with its metadata taken out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stripped {
    pub bytes: Vec<u8>,
    pub kind: Kind,
    /// Degrees clockwise a viewer turns it (from the Exif orientation; 0 without one).
    pub rotate: u16,
    pub w: Option<u32>,
    pub h: Option<u32>,
}

/// The file isn't a whole photo of its type (cut short, or not what its first bytes claim).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Broken;

/// Takes the metadata out of `b` (a photo of type `kind`, see [`sniff`]).
pub fn strip(kind: Kind, b: &[u8]) -> Result<Stripped, Broken> {
    let s = match kind {
        Kind::Jpeg => strip_jpeg(b),
        Kind::Png => strip_png(b),
        Kind::Webp => strip_webp(b),
    }?;
    // RED PROOF ONLY: the walk runs (size, turn) but the original bytes are kept
    Ok(Stripped { bytes: b.to_vec(), ..s })
}

/// Degrees clockwise for an Exif orientation. Mirrored ones (2, 4, 5, 7) keep their turn; the
/// mirror itself is lost, which nobody notices on a party photo.
fn degrees(orientation: u16) -> u16 {
    match orientation {
        3 | 4 => 180,
        5 | 6 => 90,
        7 | 8 => 270,
        _ => 0,
    }
}

/// The orientation tag (0x0112) in IFD0 of a TIFF block (Exif's body), as degrees. `None`
/// without one or when the block is malformed.
fn tiff_rotate(t: &[u8]) -> Option<u16> {
    let t = t.strip_prefix(b"Exif\0\0").unwrap_or(t);
    let le = match t.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let s = t.get(o..o.checked_add(2)?)?;
        Some(if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let s = t.get(o..o.checked_add(4)?)?;
        let a = [s[0], s[1], s[2], s[3]];
        Some(if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let ifd = u32_at(4)? as usize;
    let n = u16_at(ifd)? as usize;
    for k in 0..n.min(1024) {
        let e = ifd.checked_add(2 + k * 12)?;
        if u16_at(e)? == 0x0112 {
            return Some(degrees(u16_at(e + 8)?));
        }
    }
    None
}

/// The Exif orientation for degrees clockwise (1 for none or anything else).
fn orientation(rotate: u16) -> u16 {
    match rotate {
        90 => 6,
        180 => 3,
        270 => 8,
        _ => 1,
    }
}

/// For the zip only: a stripped JPEG with one Exif block that holds the orientation and no other
/// tag, right after the start marker (after JFIF when it leads), so photo apps turn it the way
/// the wall does. Stored and served bytes never get it: viewers turn by `rotate`, so it would turn
/// twice. `None` when there is nothing to turn or `b` isn't a JPEG.
pub fn jpeg_with_orientation(b: &[u8], rotate: u16) -> Option<Vec<u8>> {
    let o = orientation(rotate);
    if o == 1 || !b.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2;
    if b.get(2..4) == Some(&[0xFF, 0xE0][..]) && b.get(6..11) == Some(&b"JFIF\0"[..]) {
        at = 4 + be16(b, 4)? as usize;
        if at > b.len() {
            return None;
        }
    }
    // APP1: "Exif\0\0", a big-endian TIFF header, IFD0 with the one entry (0x0112, SHORT, 1), no next IFD
    let mut seg = vec![0xFF, 0xE1, 0x00, 0x22];
    seg.extend_from_slice(b"Exif\0\0MM\0\x2A\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01");
    seg.extend_from_slice(&o.to_be_bytes());
    seg.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let mut out = Vec::with_capacity(b.len() + seg.len());
    out.extend_from_slice(&b[..at]);
    out.extend_from_slice(&seg);
    out.extend_from_slice(&b[at..]);
    Some(out)
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn strip_jpeg(b: &[u8]) -> Result<Stripped, Broken> {
    if !b.starts_with(&[0xFF, 0xD8]) {
        return Err(Broken);
    }
    let mut out = Vec::with_capacity(b.len());
    out.extend_from_slice(&[0xFF, 0xD8]);
    let (mut rotate, mut w, mut h) = (0, None, None);
    let (mut saw_scan, mut saw_end) = (false, false);
    let mut i = 2;
    while i < b.len() {
        if b[i] != 0xFF {
            return Err(Broken);
        }
        // Fill bytes before a marker
        while i < b.len() && b[i] == 0xFF {
            i += 1;
        }
        let m = *b.get(i).ok_or(Broken)?;
        i += 1;
        match m {
            0xD9 => {
                out.extend_from_slice(&[0xFF, 0xD9]);
                saw_end = true;
                // Whatever follows the end marker (a second picture, a maker's trailer) stays out
                break;
            }
            0x01 | 0xD0..=0xD7 => {
                out.extend_from_slice(&[0xFF, m]);
                continue;
            }
            0x00 | 0xD8 => return Err(Broken),
            _ => {}
        }
        let len = be16(b, i).ok_or(Broken)? as usize;
        if len < 2 || i + len > b.len() {
            return Err(Broken);
        }
        let body = &b[i + 2..i + len];
        let keep = match m {
            0xE0 => body.starts_with(b"JFIF\0"),
            0xE1 => {
                if body.starts_with(b"Exif\0\0") {
                    if let Some(r) = tiff_rotate(body) {
                        rotate = r;
                    }
                }
                false
            }
            0xE2 => body.starts_with(b"ICC_PROFILE\0"),
            0xE3..=0xEF | 0xFE => false,
            // Start of frame (not DHT C4, JPG C8 or DAC CC): the size
            0xC0..=0xCF if !matches!(m, 0xC4 | 0xC8 | 0xCC) => {
                if body.len() >= 5 {
                    h = Some(u32::from(be16(body, 1).ok_or(Broken)?));
                    w = Some(u32::from(be16(body, 3).ok_or(Broken)?));
                }
                true
            }
            _ => true,
        };
        if keep {
            out.extend_from_slice(&[0xFF, m]);
            out.extend_from_slice(&b[i..i + len]);
        }
        i += len;
        if m == 0xDA {
            saw_scan = true;
            // The scan's data runs to the next marker that is neither a stuffed 0xFF nor a restart
            let start = i;
            loop {
                let (x, y) = (*b.get(i).ok_or(Broken)?, *b.get(i + 1).ok_or(Broken)?);
                if x == 0xFF && y != 0x00 && !(0xD0..=0xD7).contains(&y) && y != 0xFF {
                    break;
                }
                i += if x == 0xFF && y != 0xFF { 2 } else { 1 };
            }
            out.extend_from_slice(&b[start..i]);
        }
    }
    if !saw_scan || !saw_end {
        return Err(Broken);
    }
    Ok(Stripped { bytes: out, kind: Kind::Jpeg, rotate, w, h })
}

const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The PNG chunks that get through: the image, its colour and the APNG frames. Nothing else.
pub const PNG_KEEP: [&[u8; 4]; 15] =
    [b"IHDR", b"PLTE", b"tRNS", b"IDAT", b"IEND", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"sBIT", b"bKGD", b"pHYs", b"acTL", b"fcTL", b"fdAT"];

/// The WebP chunks that get through. Nothing else.
pub const WEBP_KEEP: [&[u8; 4]; 7] = [b"VP8X", b"VP8 ", b"VP8L", b"ALPH", b"ANIM", b"ANMF", b"ICCP"];

fn strip_png(b: &[u8]) -> Result<Stripped, Broken> {
    if !b.starts_with(PNG_SIG) {
        return Err(Broken);
    }
    let mut out = Vec::with_capacity(b.len());
    out.extend_from_slice(PNG_SIG);
    let (mut rotate, mut w, mut h) = (0, None, None);
    let mut i = PNG_SIG.len();
    let mut first = true;
    loop {
        let head = b.get(i..i + 8).ok_or(Broken)?;
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let ty = &head[4..8];
        let end = i.checked_add(12).and_then(|x| x.checked_add(len)).ok_or(Broken)?;
        if end > b.len() {
            return Err(Broken);
        }
        let data = &b[i + 8..i + 8 + len];
        if first && (ty != b"IHDR" || len < 8) {
            return Err(Broken);
        }
        first = false;
        let keep = match ty {
            b"IHDR" => {
                w = Some(u32::from_be_bytes([data[0], data[1], data[2], data[3]]));
                h = Some(u32::from_be_bytes([data[4], data[5], data[6], data[7]]));
                true
            }
            b"eXIf" => {
                if let Some(r) = tiff_rotate(data) {
                    rotate = r;
                }
                false
            }
            t => PNG_KEEP.iter().any(|k| &k[..] == t),
        };
        if keep {
            out.extend_from_slice(&b[i..end]);
        }
        i = end;
        if ty == b"IEND" {
            break;
        }
    }
    Ok(Stripped { bytes: out, kind: Kind::Png, rotate, w, h })
}

fn le24(b: &[u8]) -> u32 {
    u32::from(b[0]) | (u32::from(b[1]) << 8) | (u32::from(b[2]) << 16)
}

/// VP8X flags: an ICC chunk, alpha, an EXIF chunk, an XMP chunk, an animation.
const VP8X_ICC: u8 = 0x20;
const VP8X_ALPHA: u8 = 0x10;
const VP8X_ANIM: u8 = 0x02;

/// One animation frame's payload (`ANMF`): its 16-byte header, then only its alpha and image
/// chunks; any other chunk inside a frame is dropped too.
fn webp_frame(d: &[u8]) -> Result<Vec<u8>, Broken> {
    let head = d.get(..16).ok_or(Broken)?;
    let mut out = head.to_vec();
    let mut i = 16;
    while i < d.len() {
        let h = d.get(i..i + 8).ok_or(Broken)?;
        let size = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
        let data_end = (i + 8).checked_add(size).ok_or(Broken)?;
        if data_end > d.len() {
            return Err(Broken);
        }
        let next = (data_end + (size & 1)).min(d.len());
        if matches!(&h[..4], b"ALPH" | b"VP8 " | b"VP8L") {
            out.extend_from_slice(&d[i..data_end]);
            if size & 1 == 1 {
                out.push(0);
            }
        }
        i = next;
    }
    Ok(out)
}

fn strip_webp(b: &[u8]) -> Result<Stripped, Broken> {
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WEBP" {
        return Err(Broken);
    }
    let riff = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
    let end = riff.checked_add(8).ok_or(Broken)?;
    if end > b.len() || riff < 4 {
        return Err(Broken);
    }
    let mut out = Vec::with_capacity(end);
    out.extend_from_slice(b"RIFF\0\0\0\0WEBP");
    let (mut rotate, mut w, mut h) = (0, None, None);
    let mut flags_at = None;
    let (mut image, mut icc, mut anim) = (false, false, false);
    let mut i = 12;
    while i < end {
        let head = b.get(i..i + 8).filter(|_| i + 8 <= end).ok_or(Broken)?;
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let data_end = (i + 8).checked_add(size).ok_or(Broken)?;
        if data_end > end {
            return Err(Broken);
        }
        let data = &b[i + 8..data_end];
        // Chunks are padded to an even size; a last chunk may end without its pad byte
        let next = (data_end + (size & 1)).min(end);
        let keep = match &head[..4] {
            b"EXIF" => {
                if let Some(r) = tiff_rotate(data) {
                    rotate = r;
                }
                false
            }
            b"ANMF" => {
                let frame = webp_frame(data)?;
                out.extend_from_slice(b"ANMF");
                out.extend_from_slice(&u32::try_from(frame.len()).map_err(|_| Broken)?.to_le_bytes());
                out.extend_from_slice(&frame);
                if frame.len() & 1 == 1 {
                    out.push(0);
                }
                image = true;
                false
            }
            b"ICCP" => {
                icc = true;
                true
            }
            b"ANIM" => {
                anim = true;
                true
            }
            b"VP8X" => {
                if size >= 10 {
                    w = Some(le24(&data[4..7]) + 1);
                    h = Some(le24(&data[7..10]) + 1);
                    flags_at = Some(out.len() + 8);
                }
                image = true;
                true
            }
            b"VP8 " => {
                if w.is_none() && size >= 10 && data[3..6] == [0x9D, 0x01, 0x2A] {
                    w = Some(u32::from(u16::from_le_bytes([data[6], data[7]]) & 0x3FFF));
                    h = Some(u32::from(u16::from_le_bytes([data[8], data[9]]) & 0x3FFF));
                }
                image = true;
                true
            }
            b"VP8L" => {
                if w.is_none() && size >= 5 && data[0] == 0x2F {
                    let bits = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                    w = Some((bits & 0x3FFF) + 1);
                    h = Some(((bits >> 14) & 0x3FFF) + 1);
                }
                image = true;
                true
            }
            t => WEBP_KEEP.iter().any(|k| &k[..] == t),
        };
        if keep {
            out.extend_from_slice(&b[i..next]);
            if next == data_end && size & 1 == 1 {
                out.push(0);
            }
        }
        i = next;
    }
    if !image {
        return Err(Broken);
    }
    if let Some(at) = flags_at {
        // Only what was kept: never EXIF or XMP, ICC and animation only with their chunk
        let keep = VP8X_ALPHA | if icc { VP8X_ICC } else { 0 } | if anim { VP8X_ANIM } else { 0 };
        out[at] &= keep;
    }
    let size = u32::try_from(out.len() - 8).map_err(|_| Broken)?;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    Ok(Stripped { bytes: out, kind: Kind::Webp, rotate, w, h })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_by_bytes_only() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Sniff::Photo(Kind::Jpeg));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Sniff::Photo(Kind::Png));
        assert_eq!(sniff(b"RIFF\x10\0\0\0WEBPVP8 "), Sniff::Photo(Kind::Webp));
        assert_eq!(sniff(b"\0\0\0\x18ftypheic\0\0\0\0"), Sniff::Heic);
        assert_eq!(sniff(b"GIF89a......"), Sniff::Other);
        assert_eq!(sniff(b""), Sniff::Other);
    }

    #[test]
    fn a_cut_short_jpeg_is_broken() {
        assert_eq!(strip(Kind::Jpeg, &[0xFF, 0xD8, 0xFF, 0xE1, 0x00]), Err(Broken));
        assert_eq!(strip(Kind::Jpeg, &[0xFF, 0xD8, 0xFF, 0xD9]), Err(Broken), "no scan");
        assert_eq!(strip(Kind::Png, b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR"), Err(Broken));
        assert_eq!(strip(Kind::Webp, b"RIFF\xff\0\0\0WEBP"), Err(Broken));
    }

    fn png_chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = (data.len() as u32).to_be_bytes().to_vec();
        c.extend_from_slice(ty);
        c.extend_from_slice(data);
        c.extend_from_slice(&[0, 0, 0, 0]);
        c
    }

    fn riff(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = ty.to_vec();
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(data);
        if data.len() % 2 == 1 {
            c.push(0);
        }
        c
    }

    fn webp_file(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = chunks.concat();
        let mut w = b"RIFF".to_vec();
        w.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
        w.extend_from_slice(b"WEBP");
        w.extend_from_slice(&body);
        w
    }

    fn has(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn only_listed_chunks_get_through() {
        let mut p = PNG_SIG.to_vec();
        p.extend(png_chunk(b"IHDR", &[0, 0, 0, 3, 0, 0, 0, 2, 8, 2, 0, 0, 0]));
        p.extend(png_chunk(b"iCCP", b"ICC-KEPT\0\0x"));
        p.extend(png_chunk(b"caBX", b"C2PA-CANARY"));
        p.extend(png_chunk(b"prVt", b"PRIVATE-CANARY"));
        p.extend(png_chunk(b"IDAT", &[1, 2, 3]));
        p.extend(png_chunk(b"IEND", &[]));
        let s = strip(Kind::Png, &p).unwrap();
        assert!(!has(&s.bytes, b"CANARY") && !has(&s.bytes, b"caBX") && !has(&s.bytes, b"prVt"));
        assert!(has(&s.bytes, b"ICC-KEPT"), "ICC stays on purpose");

        // VP8X says ICC, alpha, EXIF, XMP and animation; there is ICC and C2PA but no ANIM
        let mut vp8x = vec![0x20 | 0x10 | 0x08 | 0x04 | 0x02, 0, 0, 0];
        vp8x.extend_from_slice(&[0x1F, 0x03, 0x00, 0x57, 0x02, 0x00]);
        let w = webp_file(&[riff(b"VP8X", &vp8x), riff(b"ICCP", b"ICC-KEPT"), riff(b"VP8L", &[0x2F, 1, 2, 3, 4]), riff(b"C2PA", b"C2PA-CANARY!")]);
        let s = strip(Kind::Webp, &w).unwrap();
        assert!(!has(&s.bytes, b"CANARY") && !has(&s.bytes, b"C2PA"));
        assert!(has(&s.bytes, b"ICC-KEPT"));
        assert_eq!(s.bytes[20], 0x20 | 0x10, "flags match the kept chunks");
        assert_eq!(u32::from_le_bytes([s.bytes[4], s.bytes[5], s.bytes[6], s.bytes[7]]) as usize, s.bytes.len() - 8);
        assert_eq!((s.w, s.h), (Some(800), Some(600)));

        // No ICCP kept: the ICC flag goes too. A frame keeps its image, not what rides inside it
        let mut frame = vec![0u8; 16];
        frame.extend(riff(b"VP8L", &[0x2F, 1, 2, 3, 4]));
        frame.extend(riff(b"XYZW", b"FRAME-CANARY"));
        let w = webp_file(&[riff(b"VP8X", &vp8x), riff(b"ANIM", &[0; 6]), riff(b"ANMF", &frame), riff(b"EXIF", b"EXIF-CANARY")]);
        let s = strip(Kind::Webp, &w).unwrap();
        assert!(!has(&s.bytes, b"CANARY") && !has(&s.bytes, b"XYZW"));
        assert_eq!(s.bytes[20], 0x10 | 0x02);
        assert_eq!(u32::from_le_bytes([s.bytes[4], s.bytes[5], s.bytes[6], s.bytes[7]]) as usize, s.bytes.len() - 8);
        let at = s.bytes.windows(4).position(|x| x == b"ANMF").unwrap();
        let size = u32::from_le_bytes([s.bytes[at + 4], s.bytes[at + 5], s.bytes[at + 6], s.bytes[at + 7]]) as usize;
        assert_eq!(size, 16 + 8 + 5 + 1, "the frame's size is rewritten");
    }

    #[test]
    fn the_zip_orientation_block_holds_the_turn_and_nothing_else() {
        let jfif = [0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0];
        let mut j = vec![0xFF, 0xD8];
        j.extend_from_slice(&jfif);
        j.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0x12, 0xFF, 0xD9]);
        assert_eq!(jpeg_with_orientation(&j, 0), None);
        assert_eq!(jpeg_with_orientation(b"\x89PNG", 90), None);
        for (rotate, o) in [(90u16, 6u16), (180, 3), (270, 8)] {
            let z = jpeg_with_orientation(&j, rotate).unwrap();
            assert_eq!(&z[..20], &j[..20], "after SOI and JFIF");
            assert_eq!(&z[20..24], &[0xFF, 0xE1, 0x00, 0x22]);
            assert_eq!(z.len(), j.len() + 36);
            assert_eq!(&z[56..], &j[20..]);
            assert_eq!(tiff_rotate(&z[24..56]), Some(rotate));
            assert_eq!(&z[24 + 6 + 8 + 2 + 8..24 + 6 + 8 + 2 + 10], &o.to_be_bytes());
            // Stripping it again gives the same turn back and no Exif
            let again = strip(Kind::Jpeg, &z).unwrap();
            assert_eq!(again.rotate, rotate);
            assert_eq!(again.bytes, j);
        }
        let bare = [0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02, 0x12, 0xFF, 0xD9];
        assert_eq!(&jpeg_with_orientation(&bare, 90).unwrap()[2..4], &[0xFF, 0xE1], "right after SOI without JFIF");
    }
}
