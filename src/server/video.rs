//! Looking inside videos for `read_attachment` (video-drop-073 V5): ffprobe for length, size and
//! codec, then ffmpeg for up to six evenly spaced JPEG frames the bot sees as images.
//!
//! The file is untrusted media, so ffmpeg is held short:
//! - input is the blob path (`file:<data>/attachments/xx/<sha>`), never the name the user gave;
//! - the demuxer is forced from the sniffed type (`-f mov -enable_drefs 0`, or `-f matroska`), so
//!   no format probing, no playlists, no external data references; `-protocol_whitelist file`;
//! - `-max_pixels` refuses frames with absurd dimensions, `-threads 2`, `ffmpeg -nostdin` (ffprobe
//!   has no such flag: it gets a closed stdin like ffmpeg);
//! - an empty environment, its own session and process group, `nice 10`, `RLIMIT_FSIZE` and
//!   `RLIMIT_CORE` 0, as the git reader (`dev::git`), plus a 2 GB memory cap (`RLIMIT_AS`) on
//!   Linux only: macOS can't lower it, so there it is tried and skipped;
//! - output only into a fresh temp dir that is removed however the read ends;
//! - one deadline (20 s, as a PDF) for the whole read; on it the process group is killed;
//! - at most two video reads at once.
//!
//! Without ffmpeg/ffprobe the bot gets the size and [`NO_FFMPEG`].

use serde_json::{json, Value};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};

/// Longest a whole video read (probe and frames) may take.
pub const READ_SECS: u64 = 20;
/// The line a bot gets when the server has no ffmpeg.
pub const NO_FFMPEG: &str = "Install ffmpeg on the server to let bots look inside videos.";
/// Windows (0.7.6 beta): ffmpeg can't be held short there (no setsid, nice or rlimits), so bots
/// get the size and this line instead.
pub const NOT_ON_WINDOWS: &str = "Looking inside videos is not available on Windows yet.";
/// Why a read stopped when ffmpeg or ffprobe couldn't be started.
const NO_START: &str = "ffmpeg couldn't start on this server.";
const MAX_FRAMES: usize = 6;
/// Longest side of a frame the bot gets.
const FRAME_EDGE: u32 = 1280;
/// Largest frame returned (the `read_attachment` image cap).
const MAX_FRAME_BYTES: u64 = 5 * 1024 * 1024;
/// Frames bigger than this many pixels aren't decoded (8K is 33 MP).
const MAX_PIXELS: &str = "40000000";
/// No file ffmpeg writes may pass this.
const OUT_FSIZE: u64 = 16 * 1024 * 1024;
/// Address space for one ffmpeg/ffprobe.
const MAX_AS: u64 = 2 * 1024 * 1024 * 1024;
/// Most ffprobe JSON read.
const MAX_PROBE_OUT: usize = 256 * 1024;
/// Video reads at once: decoding is CPU- and memory-heavy, and the server is small.
static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

/// Where ffprobe and ffmpeg live.
#[derive(Debug, Clone)]
pub struct Tools {
    pub ffprobe: PathBuf,
    pub ffmpeg: PathBuf,
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Both tools from the absolute directories of `path_var`, then `extra` (a Mac app's PATH lacks
/// Homebrew). `None` unless both are found.
pub fn find_tools(path_var: Option<&OsStr>, extra: &[&Path]) -> Option<Tools> {
    let mut dirs: Vec<PathBuf> = path_var.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    dirs.extend(extra.iter().map(|p| p.to_path_buf()));
    // A relative entry ("." or "") would pick up a program from wherever the server runs
    dirs.retain(|d| d.is_absolute());
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let find = |name: &str| dirs.iter().map(|d| d.join(format!("{name}{exe}"))).find(|p| is_executable(p));
    Some(Tools { ffprobe: find("ffprobe")?, ffmpeg: find("ffmpeg")? })
}

/// The server's ffmpeg, if it has one.
pub fn tools() -> Option<Tools> {
    find_tools(
        std::env::var_os("PATH").as_deref(),
        &[Path::new("/opt/homebrew/bin"), Path::new("/usr/local/bin"), Path::new("/opt/local/bin"), Path::new("/usr/bin")],
    )
}

/// What ffprobe says about a video.
#[derive(Debug, Default, PartialEq)]
pub struct Info {
    pub duration: Option<f64>,
    /// As shown: width and height swapped for a video recorded on its side.
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub codec: Option<String>,
    pub fps: Option<f64>,
    pub audio: Option<String>,
}

fn ratio(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/')?;
    let (n, d) = (n.trim().parse::<f64>().ok()?, d.trim().parse::<f64>().ok()?);
    (d > 0.0 && n > 0.0).then(|| n / d)
}

/// The facts in ffprobe's JSON; `None` without a video stream.
pub fn parse_probe(out: &[u8]) -> Option<Info> {
    let v: Value = serde_json::from_slice(out).ok()?;
    let streams = v["streams"].as_array()?;
    let video = streams.iter().find(|s| s["codec_type"] == "video")?;
    let mut info = Info {
        duration: v["format"]["duration"].as_str().and_then(|d| d.parse::<f64>().ok()).filter(|d| d.is_finite() && *d > 0.0),
        width: video["width"].as_u64().filter(|w| *w > 0),
        height: video["height"].as_u64().filter(|h| *h > 0),
        codec: video["codec_name"].as_str().map(|c| c.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(24).collect()),
        fps: video["avg_frame_rate"].as_str().and_then(ratio).filter(|f| *f < 1000.0),
        audio: streams
            .iter()
            .find(|s| s["codec_type"] == "audio")
            .map(|a| a["codec_name"].as_str().unwrap_or("yes").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(24).collect()),
    };
    let rotation = video["side_data_list"]
        .as_array()
        .and_then(|l| l.iter().find_map(|d| d["rotation"].as_f64()))
        .or_else(|| video["tags"]["rotate"].as_str().and_then(|r| r.parse::<f64>().ok()))
        .unwrap_or(0.0);
    if (rotation.abs() - 90.0).abs() < 1.0 || (rotation.abs() - 270.0).abs() < 1.0 {
        std::mem::swap(&mut info.width, &mut info.height);
    }
    Some(info)
}

/// `m:ss.s` (or `h:mm:ss`).
fn clock(secs: f64) -> String {
    let s = secs.max(0.0);
    if s >= 3600.0 {
        let t = s as u64;
        return format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60);
    }
    let m = (s / 60.0).floor();
    format!("{}:{:04.1}", m as u64, s - m * 60.0)
}

pub fn human_size(size: i64) -> String {
    let s = size.max(0) as f64;
    if s >= 1024.0 * 1024.0 {
        format!("{:.1} MB", s / 1024.0 / 1024.0)
    } else {
        format!("{} KB", ((s + 1023.0) / 1024.0).floor() as u64)
    }
}

fn summary(info: &Info) -> String {
    let mut parts = vec![match info.duration {
        Some(d) => format!("Duration {}", clock(d)),
        None => "Duration unknown".to_string(),
    }];
    if let (Some(w), Some(h)) = (info.width, info.height) {
        parts.push(format!("{w}×{h}"));
    }
    if let Some(c) = &info.codec {
        parts.push(c.clone());
    }
    if let Some(f) = info.fps {
        parts.push(if (f - f.round()).abs() < 0.01 { format!("{} fps", f.round() as u64) } else { format!("{f:.2} fps") });
    }
    parts.push(match &info.audio {
        Some(a) => format!("audio {a}"),
        None => "no audio".to_string(),
    });
    parts.join(" · ")
}

/// Where frames are taken: the middles of `n` equal slices, `n` = whole seconds up to 6 (at
/// least 1); one at the start when the length is unknown.
pub fn frame_times(duration: Option<f64>) -> Vec<f64> {
    let Some(d) = duration else { return vec![0.0] };
    let n = (d.floor() as usize).clamp(1, MAX_FRAMES);
    (0..n).map(|i| d * (2 * i + 1) as f64 / (2 * n) as f64).collect()
}

/// The input options for the sniffed type: the demuxer is never guessed from the bytes.
fn input_opts(mime: &str) -> Vec<&'static str> {
    let mut o = vec!["-protocol_whitelist", "file"];
    if mime == "video/webm" {
        o.extend(["-f", "matroska"]);
    } else {
        o.extend(["-f", "mov", "-enable_drefs", "0"]);
    }
    o.extend(["-max_pixels", MAX_PIXELS]);
    o
}

#[derive(Debug, PartialEq)]
enum RunError {
    Timeout,
    /// It ran and failed (or wrote too much): blame the file.
    Failed,
    /// It never started (missing, not executable, missing libraries, a limit refused): blame
    /// the server, not the file.
    Spawn,
}

/// A temp dir removed when dropped, however the read ends.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn read_capped<R: AsyncRead + Unpin>(r: Option<R>, cap: usize) -> Result<Vec<u8>, RunError> {
    let Some(mut r) = r else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let n = r.read(&mut buf).await.map_err(|_| RunError::Failed)?;
        if n == 0 {
            return Ok(out);
        }
        if out.len() + n > cap {
            return Err(RunError::Failed);
        }
        out.extend_from_slice(&buf[..n]);
    }
}

/// Runs one ffmpeg/ffprobe as described in the module docs; its stdout on success.
async fn run(bin: &Path, args: &[OsString], cwd: &Path, until: Instant) -> Result<Vec<u8>, RunError> {
    let left = until.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or(RunError::Timeout)?;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args)
        .env_clear()
        .env("LC_ALL", "C")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    // SAFETY: setsid(2), nice(2), setrlimit(2) and signal(2) are async-signal-safe and touch no
    // memory of this process
    unsafe {
        cmd.pre_exec(|| {
            // Its own process group, so a timeout kills everything it started
            libc::setsid();
            libc::nice(10);
            let limit = |res, cap: u64| {
                let lim = libc::rlimit { rlim_cur: cap as libc::rlim_t, rlim_max: cap as libc::rlim_t };
                libc::setrlimit(res, &lim)
            };
            // Required everywhere, as in dev::git: no ffmpeg runs without them
            for (res, cap) in [(libc::RLIMIT_FSIZE, OUT_FSIZE), (libc::RLIMIT_CORE, 0)] {
                if limit(res, cap) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            // The memory cap is required on Linux, where RLIMIT_AS is enforced. macOS rejects
            // lowering it (EINVAL; Darwin maps it to RSS and doesn't enforce it), and Mac is the
            // main platform, so elsewhere it is best-effort: a refusal must not stop every video
            // read. The 20 s deadline, -max_pixels and -threads 2 still bound the work there.
            #[cfg(target_os = "linux")]
            if limit(libc::RLIMIT_AS, MAX_AS) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            #[cfg(not(target_os = "linux"))]
            let _ = limit(libc::RLIMIT_AS, MAX_AS);
            libc::signal(libc::SIGXFSZ, libc::SIG_DFL);
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|_| RunError::Spawn)?;
    let mut group = crate::drivers::cli::ProcessGroup::of(&child);
    let out = child.stdout.take();
    let work = async {
        let out = read_capped(out, MAX_PROBE_OUT).await?;
        let status = child.wait().await.map_err(|_| RunError::Failed)?;
        Ok::<_, RunError>((out, status))
    };
    let (out, status) = tokio::time::timeout(left, work).await.map_err(|_| RunError::Timeout)??;
    group.disarm();
    if status.success() {
        Ok(out)
    } else {
        Err(RunError::Failed)
    }
}

fn file_arg(blob: &Path) -> OsString {
    let mut a = OsString::from("file:");
    a.push(blob.as_os_str());
    a
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// `read_attachment` for a video: MCP content blocks, and whether it is an error.
pub async fn describe(name: &str, mime: &str, size: i64, blob: &Path, tmp_parent: &Path) -> (Value, bool) {
    if cfg!(windows) {
        return (json!([{"type": "text", "text": format!("{name} is a video ({mime}, {}). {NOT_ON_WINDOWS}", human_size(size))}]), false);
    }
    describe_with(tools(), name, mime, size, blob, tmp_parent, Duration::from_secs(READ_SECS)).await
}

pub async fn describe_with(
    tools: Option<Tools>,
    name: &str,
    mime: &str,
    size: i64,
    blob: &Path,
    tmp_parent: &Path,
    limit: Duration,
) -> (Value, bool) {
    let text = |t: String| json!([{"type": "text", "text": t}]);
    let label = format!("{name} ({mime}, {})", human_size(size));
    let Some(tools) = tools else {
        return (text(format!("{name} is a video ({mime}, {}). {NO_FFMPEG}", human_size(size))), false);
    };
    let Ok(_slot) = SLOTS.acquire().await else {
        return (text(format!("Couldn't look inside {label}: the server is shutting down.")), true);
    };
    let until = Instant::now() + limit;
    let dir = tmp_parent.join(format!("video-{}", uuid::Uuid::new_v4()));
    if tokio::fs::create_dir_all(&dir).await.is_err() {
        return (text(format!("Couldn't look inside {label}: no room for a temp folder.")), true);
    }
    let dir = TempDir(dir);
    let input = file_arg(blob);

    let mut probe = os(&["-v", "error", "-hide_banner"]);
    probe.extend(os(&input_opts(mime)));
    probe.extend(os(&[
        "-show_entries",
        "format=duration:stream=codec_type,codec_name,width,height,avg_frame_rate:stream_side_data=rotation:stream_tags=rotate",
        "-of",
        "json",
    ]));
    probe.push(input.clone());
    let info = match run(&tools.ffprobe, &probe, &dir.0, until).await.map(|o| parse_probe(&o)) {
        Ok(Some(info)) => info,
        Ok(None) | Err(RunError::Failed) => {
            return (text(format!("Couldn't look inside {label}: ffmpeg can't read it (it may be cut off or damaged).")), true)
        }
        Err(RunError::Timeout) => {
            return (text(format!("Couldn't look inside {label}: reading it took longer than {} seconds.", limit.as_secs())), true)
        }
        Err(RunError::Spawn) => return (text(format!("Couldn't look inside {label}: {NO_START}")), true),
    };

    let times = frame_times(info.duration);
    let scale = format!("scale=w='min({FRAME_EDGE},iw)':h='min({FRAME_EDGE},ih)':force_original_aspect_ratio=decrease");
    let mut frames: Vec<(f64, Vec<u8>)> = Vec::new();
    let mut timed_out = false;
    let mut no_start = false;
    for (i, t) in times.iter().enumerate() {
        let out = dir.0.join(format!("f{i}.jpg"));
        let mut args = os(&["-nostdin", "-v", "error", "-hide_banner", "-threads", "2"]);
        args.extend(os(&input_opts(mime)));
        args.extend(os(&["-ss", &format!("{t:.3}"), "-i"]));
        args.push(input.clone());
        args.extend(os(&["-map", "0:v:0", "-frames:v", "1", "-an", "-sn", "-dn", "-vf", &scale, "-q:v", "5", "-f", "image2", "-update", "1", "-y"]));
        args.push(out.clone().into_os_string());
        match run(&tools.ffmpeg, &args, &dir.0, until).await {
            Ok(_) => {}
            Err(RunError::Timeout) => {
                timed_out = true;
                break;
            }
            Err(RunError::Failed) => continue,
            // ffprobe ran but ffmpeg can't: the same for every frame
            Err(RunError::Spawn) => {
                no_start = true;
                break;
            }
        }
        if let Ok(meta) = tokio::fs::metadata(&out).await {
            if meta.len() > 0 && meta.len() <= MAX_FRAME_BYTES {
                if let Ok(bytes) = tokio::fs::read(&out).await {
                    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                        frames.push((*t, bytes));
                    }
                }
            }
        }
    }
    drop(dir);

    let mut head = format!("--- {label} ---\n{}", summary(&info));
    head.push_str(&match (frames.len(), timed_out) {
        (0, true) => format!("\nNo frames: making them took longer than {} seconds.", limit.as_secs()),
        (0, false) if no_start => format!("\nNo frames: {NO_START}"),
        (0, false) => "\nNo frames could be taken from it.".to_string(),
        (n, true) => format!("\n{n} of {} frames (the rest took longer than {} seconds):", times.len(), limit.as_secs()),
        (1, false) => "\n1 frame:".to_string(),
        (n, false) => format!("\n{n} frames, evenly spaced:"),
    });
    let mut blocks = vec![json!({"type": "text", "text": head})];
    use base64::Engine;
    for (i, (t, bytes)) in frames.iter().enumerate() {
        blocks.push(json!({"type": "text", "text": format!("frame {} at {}", i + 1, clock(*t))}));
        blocks.push(json!({"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(bytes), "mimeType": "image/jpeg"}));
    }
    (Value::Array(blocks), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_json_reads_rotation_audio_and_rates() {
        let j = br#"{"streams":[{"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"avg_frame_rate":"30000/1001","side_data_list":[{"rotation":-90}]},{"codec_type":"audio","codec_name":"aac"}],"format":{"duration":"12.400000"}}"#;
        let i = parse_probe(j).unwrap();
        assert_eq!((i.width, i.height), (Some(1080), Some(1920)), "a phone video on its side shows upright");
        assert_eq!(i.duration, Some(12.4));
        assert_eq!(summary(&i), "Duration 0:12.4 · 1080×1920 · h264 · 29.97 fps · audio aac");
        let none = br#"{"streams":[{"codec_type":"audio","codec_name":"aac"}],"format":{}}"#;
        assert!(parse_probe(none).is_none(), "no video stream");
        assert!(parse_probe(b"not json").is_none());
        let bare = parse_probe(br#"{"streams":[{"codec_type":"video","codec_name":"vp9<script>","avg_frame_rate":"0/0"}],"format":{"duration":"N/A"}}"#).unwrap();
        assert_eq!(summary(&bare), "Duration unknown · vp9script · no audio");
    }

    #[test]
    fn frames_are_evenly_spaced_and_at_most_six() {
        assert_eq!(frame_times(None), vec![0.0]);
        assert_eq!(frame_times(Some(0.4)), vec![0.2]);
        assert_eq!(frame_times(Some(2.5)).len(), 2);
        let t = frame_times(Some(60.0));
        assert_eq!(t, vec![5.0, 15.0, 25.0, 35.0, 45.0, 55.0]);
        assert_eq!(clock(65.25), "1:05.2");
        assert_eq!(clock(3725.0), "1:02:05");
    }

    #[test]
    fn tools_come_from_absolute_dirs_only() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(find_tools(Some(empty.as_os_str()), &[]).is_none(), "PATH without ffmpeg");
        assert!(find_tools(None, &[]).is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let bin = tmp.path().join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            for n in ["ffprobe", "ffmpeg"] {
                std::fs::write(bin.join(n), "#!/bin/sh\n").unwrap();
                std::fs::set_permissions(bin.join(n), std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let path = std::env::join_paths([empty.clone(), bin.clone()]).unwrap();
            assert_eq!(find_tools(Some(&path), &[]).unwrap().ffmpeg, bin.join("ffmpeg"));
            // Relative entries are skipped even when they hold the tools
            let rel = std::env::join_paths([PathBuf::from("."), PathBuf::from("bin")]).unwrap();
            assert!(find_tools(Some(&rel), &[]).is_none());
            std::fs::set_permissions(bin.join("ffmpeg"), std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(find_tools(Some(&path), &[]).is_none(), "not executable");
        }
    }

    #[tokio::test]
    async fn without_ffmpeg_the_bot_gets_the_size_and_one_line() {
        let tmp = tempfile::tempdir().unwrap();
        let (v, err) =
            describe_with(None, "clip.mp4", "video/mp4", 8 * 1024 * 1024, &tmp.path().join("x"), tmp.path(), Duration::from_secs(5)).await;
        assert!(!err);
        assert_eq!(v[0]["text"], format!("clip.mp4 is a video (video/mp4, 8.0 MB). {NO_FFMPEG}"));
        assert_eq!(v.as_array().unwrap().len(), 1);
    }

    /// A binary that can't be started (gone since the lookup, not executable, missing libraries)
    /// is the server's problem: the bot isn't told the file is damaged.
    #[tokio::test]
    async fn an_ffmpeg_that_cannot_start_is_not_blamed_on_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let blob = tmp.path().join("blob");
        std::fs::write(&blob, b"\0\0\0\x20ftypisom").unwrap();
        let not_exec = tmp.path().join("ffprobe");
        std::fs::write(&not_exec, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&not_exec, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        for ffprobe in [tmp.path().join("missing/ffprobe"), not_exec] {
            let tools = Tools { ffprobe: ffprobe.clone(), ffmpeg: tmp.path().join("missing/ffmpeg") };
            let (v, err) = describe_with(Some(tools), "clip.mp4", "video/mp4", 2048, &blob, tmp.path(), Duration::from_secs(5)).await;
            assert!(err, "{ffprobe:?}");
            assert_eq!(v[0]["text"], "Couldn't look inside clip.mp4 (video/mp4, 2 KB): ffmpeg couldn't start on this server.");
        }
        let left: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("video-")).collect();
        assert!(left.is_empty(), "temp dirs removed");
    }

    /// A hostile file that makes ffprobe (or ffmpeg) hang: the read stops at its deadline, the
    /// whole process group is killed and the temp dir is gone.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_hanging_ffmpeg_is_stopped_at_the_deadline_and_killed() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let pidfile = tmp.path().join("pid");
        // The fake tool forks a child that sleeps, so the kill has to reach the whole group
        let script = format!("#!/bin/sh\nsleep 60 &\necho $! > {}\nwait\n", pidfile.display());
        for n in ["ffprobe", "ffmpeg"] {
            std::fs::write(bin.join(n), &script).unwrap();
            std::fs::set_permissions(bin.join(n), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let tools = Tools { ffprobe: bin.join("ffprobe"), ffmpeg: bin.join("ffmpeg") };
        let blob = tmp.path().join("blob");
        std::fs::write(&blob, b"\0\0\0\x20ftypisom").unwrap();
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let start = Instant::now();
        let (v, err) = describe_with(Some(tools), "evil.mp4", "video/mp4", 16, &blob, &work, Duration::from_secs(1)).await;
        assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
        assert!(err);
        assert!(v[0]["text"].as_str().unwrap().contains("took longer than 1 seconds"), "{v}");
        assert_eq!(std::fs::read_dir(&work).unwrap().count(), 0, "temp dir removed");
        let pid: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        // SIGKILL to the group is asynchronous; give the kernel a moment
        let mut gone = false;
        for _ in 0..50 {
            // SAFETY: kill(2) with signal 0 only checks the pid
            if unsafe { libc::kill(pid, 0) } != 0 {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(gone, "the sleeping grandchild was killed with the group");
    }
}
