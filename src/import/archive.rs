//! Reading containers in memory, defensively: zips (no paths outside the archive, no nested
//! archives, inflated size counted as it is read) and the text chunks of character-card PNGs.

use super::{ImportError, MAX_FILE_BYTES, MAX_UNZIPPED_BYTES, MAX_ZIP_ENTRIES};
use base64::Engine;
use std::io::{Cursor, Read};

/// One file read out of a zip.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// What a zip held: the files kwant may read, and what it left alone.
#[derive(Debug, Default)]
pub struct Unzipped {
    pub entries: Vec<Entry>,
    pub skipped: Vec<String>,
}

const READ_EXTENSIONS: [&str; 4] = [".json", ".md", ".png", ".txt"];
const ARCHIVE_EXTENSIONS: [&str; 9] = [".zip", ".gz", ".tgz", ".tar", ".7z", ".rar", ".xz", ".bz2", ".zst"];
/// Chat history never comes in (v1), so these aren't even inflated.
const CHAT_FILES: [&str; 3] = ["conversations.json", "chat.html", "conversations.html"];

fn invalid(msg: &str) -> ImportError {
    ImportError::Invalid(msg.into())
}

/// A path stays inside the archive: relative, no `..`, no drive letters.
fn safe_path(name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    if name.starts_with('/') || name.contains(':') || name.contains('\0') {
        return None;
    }
    if name.split('/').any(|part| part == "..") {
        return None;
    }
    Some(name.trim_start_matches("./").to_string())
}

/// Reads the bot-relevant files of a zip. Rejects the whole zip for unsafe paths, nested
/// archives, encryption, too many entries, or more than [`MAX_UNZIPPED_BYTES`] once inflated.
pub fn read_zip(bytes: &[u8]) -> Result<Unzipped, ImportError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid("This zip can't be read."))?;
    if zip.len() > MAX_ZIP_ENTRIES {
        return Err(invalid(&format!("This zip has more than {MAX_ZIP_ENTRIES} files.")));
    }
    let mut out = Unzipped::default();
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|_| invalid("A file in this zip can't be read (it may be encrypted)."))?;
        let raw_name = file.name().map_err(|_| invalid("A file name in this zip can't be read."))?.to_string();
        let path = safe_path(&raw_name).ok_or_else(|| invalid("This zip has a file path that points outside it, so it wasn't opened."))?;
        if file.is_dir() {
            continue;
        }
        let lower = path.to_lowercase();
        if ARCHIVE_EXTENSIONS.iter().any(|e| lower.ends_with(e)) {
            return Err(invalid("This zip contains another archive. Unpack it and import the inner file instead."));
        }
        let base = lower.rsplit('/').next().unwrap_or(&lower);
        if base.starts_with('.') || lower.starts_with("__macosx/") || CHAT_FILES.contains(&base) || !READ_EXTENSIONS.iter().any(|e| lower.ends_with(e)) {
            out.skipped.push(path);
            continue;
        }
        // The size header can lie, so count what actually comes out.
        let mut buf = Vec::new();
        (&mut file)
            .take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut buf)
            .map_err(|_| invalid("A file in this zip is damaged."))?;
        if buf.len() > MAX_FILE_BYTES {
            return Err(ImportError::TooLarge(format!("{path} is larger than 4 MB once unpacked.")));
        }
        total += buf.len() as u64;
        if total > MAX_UNZIPPED_BYTES {
            return Err(ImportError::TooLarge("This zip is larger than 50 MB once unpacked.".into()));
        }
        out.entries.push(Entry { path, bytes: buf });
    }
    Ok(out)
}

pub const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The character card JSON in a PNG's `ccv3` (preferred) or `chara` text chunk.
pub fn card_json_from_png(bytes: &[u8]) -> Result<serde_json::Value, ImportError> {
    if !bytes.starts_with(&PNG_SIGNATURE) {
        return Err(invalid("This isn't a PNG."));
    }
    let mut pos = PNG_SIGNATURE.len();
    let (mut chara, mut ccv3) = (None, None);
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let start = pos + 8;
        let end = start.checked_add(len).filter(|e| e + 4 <= bytes.len()).ok_or_else(|| invalid("This PNG is damaged."))?;
        let data = &bytes[start..end];
        if kind == b"IEND" {
            break;
        }
        let text = match kind {
            b"tEXt" => data.iter().position(|b| *b == 0).map(|z| (&data[..z], &data[z + 1..])),
            // keyword \0 compressed-flag method language \0 translated \0 text; compressed ones are skipped
            b"iTXt" => data.iter().position(|b| *b == 0).and_then(|z| {
                let rest = data.get(z + 1..)?;
                if *rest.first()? != 0 {
                    return None;
                }
                let rest = rest.get(2..)?;
                let lang_end = rest.iter().position(|b| *b == 0)?;
                let rest = &rest[lang_end + 1..];
                let tr_end = rest.iter().position(|b| *b == 0)?;
                Some((&data[..z], &rest[tr_end + 1..]))
            }),
            _ => None,
        };
        if let Some((keyword, value)) = text {
            match keyword {
                b"ccv3" => ccv3 = Some(value),
                b"chara" => chara = Some(value),
                _ => {}
            }
        }
        pos = end + 4;
    }
    let raw = ccv3.or(chara).ok_or_else(|| ImportError::Unsupported("This PNG has no character card in it.".into()))?;
    let cleaned: Vec<u8> = raw.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&cleaned)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(&cleaned))
        .map_err(|_| invalid("The character card in this PNG isn't valid."))?;
    serde_json::from_slice(&decoded).map_err(|_| invalid("The character card in this PNG isn't valid JSON."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_paths() {
        assert_eq!(safe_path("agents/a.md").as_deref(), Some("agents/a.md"));
        assert_eq!(safe_path("../x.md"), None);
        assert_eq!(safe_path("a/../../x.md"), None);
        assert_eq!(safe_path("/etc/passwd"), None);
        assert_eq!(safe_path("C:\\x.md"), None);
        assert_eq!(safe_path("a\\..\\x.md"), None);
    }
}
