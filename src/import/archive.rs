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
    /// The archive comment (codeload writes the commit id there).
    pub comment: Option<String>,
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

/// Dotfiles kwant reads even though they're hidden: Paperclip's `.paperclip.yaml` (only its
/// `routines`) and Hermes' `skills/.bundled_manifest` (which skills Hermes seeded itself).
pub const READ_DOTFILES: [&str; 2] = [".paperclip.yaml", ".bundled_manifest"];
/// Repository archives (GitHub links) may hold this many entries; kwant still reads at most
/// [`MAX_ZIP_ENTRIES`] files out of them.
pub const MAX_REPO_ZIP_ENTRIES: usize = 5_000;

/// How a zip is read: an upload, or a repository archive fetched from a GitHub link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ZipKind {
    #[default]
    Upload,
    /// Up to [`MAX_REPO_ZIP_ENTRIES`] entries; only files a format can use are inflated (see
    /// [`wanted_in_repo`]), at most [`MAX_ZIP_ENTRIES`] of them; nested archives are skipped, not
    /// refused, since they are never opened.
    Repo,
}

/// A path part (after the first) that starts with `.`: `skills/.system/…`, `skills/.hub/…`. The
/// first may be hidden so a picked `.openclaw` or `.hermes` folder still reads.
fn in_hidden_folder(lower: &str) -> bool {
    let parts: Vec<&str> = lower.split('/').collect();
    parts.len() > 2 && parts[1..parts.len() - 1].iter().any(|p| p.starts_with('.'))
}

/// A licence file at any level: `LICENSE`, `LICENSE.md`, `LICENCE.txt`, `COPYING`, ….
pub fn is_licence_file(lower_base: &str) -> bool {
    let stem = lower_base.split('.').next().unwrap_or(lower_base);
    matches!(stem, "license" | "licence" | "copying") && (lower_base == stem || lower_base.ends_with(".md") || lower_base.ends_with(".txt"))
}

/// Files of a repository archive that some import format reads. Everything else in a repo
/// (code, docs, a skill's reference notes) is skipped without being inflated.
pub fn wanted_in_repo(lower: &str) -> bool {
    let base = lower.rsplit('/').next().unwrap_or(lower);
    const NAMES: [&str; 14] = [
        "skill.md", "agents.md", "agents.override.md", "soul.md", "identity.md", "user.md", "memory.md", "company.md",
        "team.md", "task.md", "employee.json", "schedule.md", ".paperclip.yaml", ".bundled_manifest",
    ];
    if NAMES.contains(&base) || is_licence_file(base) {
        return true;
    }
    let parent = lower.rsplit('/').nth(1).unwrap_or("");
    // Claude Code agents (`agents/*.md`), Hermes memories, kwant bundles and other JSON exports
    (parent == "agents" && base.ends_with(".md")) || (parent == "memories" && base.ends_with(".md")) || (base.ends_with(".json") && !lower.contains("node_modules/") && base != "package.json" && base != "package-lock.json" && base != "tsconfig.json")
}

/// Reads the bot-relevant files of an uploaded zip. See [`read_zip_as`].
pub fn read_zip(bytes: &[u8]) -> Result<Unzipped, ImportError> {
    read_zip_as(bytes, ZipKind::Upload)
}

/// Reads the bot-relevant files of a zip. Rejects the whole zip for unsafe paths, nested
/// archives (uploads), encryption, too many entries, or more than [`MAX_UNZIPPED_BYTES`] once
/// inflated.
pub fn read_zip_as(bytes: &[u8], kind: ZipKind) -> Result<Unzipped, ImportError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid("This zip can't be read."))?;
    let max_entries = if kind == ZipKind::Repo { MAX_REPO_ZIP_ENTRIES } else { MAX_ZIP_ENTRIES };
    if zip.len() > max_entries {
        return Err(if kind == ZipKind::Repo {
            ImportError::TooLarge(format!("This repository has more than {max_entries} files."))
        } else {
            invalid(&format!("This zip has more than {max_entries} files."))
        });
    }
    let mut out = Unzipped::default();
    let mut total: u64 = 0;
    let mut read = 0usize;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|_| invalid("A file in this zip can't be read (it may be encrypted)."))?;
        let raw_name = file.name().map_err(|_| invalid("A file name in this zip can't be read."))?.to_string();
        let path = safe_path(&raw_name).ok_or_else(|| invalid("This zip has a file path that points outside it, so it wasn't opened."))?;
        if file.is_dir() {
            continue;
        }
        let lower = path.to_lowercase();
        let base = lower.rsplit('/').next().unwrap_or(&lower);
        if ARCHIVE_EXTENSIONS.iter().any(|e| lower.ends_with(e)) {
            if kind == ZipKind::Repo {
                out.skipped.push(path);
                continue;
            }
            return Err(invalid("This zip contains another archive. Unpack it and import the inner file instead."));
        }
        let readable = READ_EXTENSIONS.iter().any(|e| lower.ends_with(e)) || READ_DOTFILES.contains(&base) || is_licence_file(base);
        let hidden = base.starts_with('.') && !READ_DOTFILES.contains(&base);
        if hidden
            || in_hidden_folder(&lower)
            || lower.starts_with("__macosx/")
            || CHAT_FILES.contains(&base)
            || !readable
            || (kind == ZipKind::Repo && !wanted_in_repo(&lower))
        {
            out.skipped.push(path);
            continue;
        }
        read += 1;
        if read > MAX_ZIP_ENTRIES {
            return Err(ImportError::TooLarge(format!("This has more than {MAX_ZIP_ENTRIES} files kwant would read.")));
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
    out.comment = std::str::from_utf8(zip.comment()).ok().map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
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
