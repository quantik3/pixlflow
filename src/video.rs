//! Video through the external FFmpeg tools (spec §11): ffprobe for facts, ffmpeg to decode/encode raw RGB
//! frames over pipes, ffplay to watch. The tools are looked up next to the program first, then in PATH.
//! On Windows they start without a console window.

use crate::Rgb;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread::JoinHandle;

pub const VIDEO_EXTS: [&str; 7] = ["mp4", "mov", "mkv", "webm", "avi", "m4v", "gif"];
const NOT_FOUND: &str = "FFmpeg was not found: put ffmpeg.exe, ffprobe.exe and ffplay.exe next to pixlflow.exe or in PATH";

pub fn is_video(path: &Path) -> bool {
    // GIF counts as a video only when animated GIFs are asked for explicitly; here it stays an image
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| VIDEO_EXTS[..6].iter().any(|v| v.eq_ignore_ascii_case(e)))
}

fn command(tool: &str) -> Command {
    let local = std::env::current_exe().ok().map(|p| p.with_file_name(format!("{tool}{}", std::env::consts::EXE_SUFFIX)));
    let mut c = Command::new(local.filter(|p| p.is_file()).unwrap_or_else(|| PathBuf::from(tool)));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

fn spawn(c: &mut Command) -> Result<Child, String> {
    c.spawn().map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { NOT_FOUND.to_string() } else { format!("cannot start FFmpeg: {e}") })
}

/// Collects a child's stderr on a thread, so a chatty tool never blocks on a full pipe.
struct Log(Option<JoinHandle<String>>);

impl Log {
    fn take(child: &mut Child) -> Log {
        Log(child.stderr.take().map(|mut e| {
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = e.read_to_string(&mut s);
                s
            })
        }))
    }
    fn text(&mut self) -> String {
        self.0.take().and_then(|h| h.join().ok()).unwrap_or_default().trim().to_string()
    }
}

#[derive(Clone, Debug)]
pub struct Info {
    /// Size of the decoded frames (after the rotation stored in the file).
    pub w: usize,
    pub h: usize,
    pub fps: f64,
    pub duration: f64,
    pub frames: u64,
}

pub fn probe(path: &Path) -> Result<Info, String> {
    let out = command("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-of", "default=noprint_wrappers=1", "-show_entries"])
        .arg("stream=width,height,avg_frame_rate,r_frame_rate,nb_frames:stream_side_data=rotation:format=duration")
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { NOT_FOUND.to_string() } else { format!("cannot start ffprobe: {e}") })?;
    if !out.status.success() {
        return Err(format!("cannot read video '{}': {}", path.display(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let get = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('=')).map(str::trim);
    let num = |k: &str| get(k).and_then(|v| v.parse::<f64>().ok());
    let rate = |k: &str| {
        let (a, b) = get(k)?.split_once('/')?;
        let (a, b) = (a.parse::<f64>().ok()?, b.parse::<f64>().ok()?);
        (a > 0.0 && b > 0.0).then_some(a / b)
    };
    let (mut w, mut h) = (num("width").unwrap_or(0.0) as usize, num("height").unwrap_or(0.0) as usize);
    if w == 0 || h == 0 {
        return Err(format!("'{}' has no video stream", path.display()));
    }
    if num("rotation").is_some_and(|r| (r as i64).rem_euclid(180) == 90) {
        std::mem::swap(&mut w, &mut h); // ffmpeg turns such frames upright while decoding
    }
    let fps = rate("avg_frame_rate").or_else(|| rate("r_frame_rate")).unwrap_or(25.0);
    let duration = num("duration").unwrap_or(0.0);
    let frames = num("nb_frames").map(|n| n as u64).unwrap_or((duration * fps).round() as u64);
    Ok(Info { w, h, fps, duration, frames })
}

/// Decoded frames of a video, from `start` seconds on.
pub struct Frames {
    child: Child,
    out: ChildStdout,
    w: usize,
    h: usize,
    log: Log,
}

impl Frames {
    pub fn open(path: &Path, info: &Info, start: f64) -> Result<Frames, String> {
        let mut c = command("ffmpeg");
        c.args(["-v", "error", "-nostdin", "-fflags", "+discardcorrupt", "-err_detect", "ignore_err"]);
        if start > 0.0 {
            c.args(["-ss", &format!("{start:.3}")]);
        }
        c.arg("-i").arg(path).args(["-map", "0:v:0", "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]);
        let mut child = spawn(c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()))?;
        let out = child.stdout.take().expect("piped");
        let log = Log::take(&mut child);
        Ok(Frames { child, out, w: info.w, h: info.h, log })
    }

    pub fn next_frame(&mut self) -> Option<Vec<Rgb>> {
        let mut buf = vec![0u8; self.w * self.h * 3];
        self.out.read_exact(&mut buf).ok()?;
        Some(buf.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect())
    }

    /// FFmpeg's error text, once the frames ran out.
    fn error(&mut self) -> String {
        let _ = self.child.wait();
        self.log.text()
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub enum Sink {
    /// New video file; audio and subtitles of `streams_from` are copied without re-encoding.
    File { out: PathBuf, streams_from: PathBuf },
    /// Live window (ffplay).
    Player,
}

struct Output {
    child: Child,
    stdin: Option<ChildStdin>,
    log: Log,
}

impl Output {
    fn open(sink: &Sink, info: &Info) -> Result<Output, String> {
        let (size, fps) = (format!("{}x{}", info.w, info.h), format!("{}", info.fps));
        let mut c = match sink {
            Sink::File { out, streams_from } => {
                let webm = out.extension().is_some_and(|e| e.eq_ignore_ascii_case("webm"));
                let mut c = command("ffmpeg");
                c.args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", &size, "-r", &fps, "-i", "-"])
                    .arg("-i")
                    .arg(streams_from)
                    .args(["-map", "0:v", "-map", "1:a?", "-map", "1:s?", "-map_metadata", "1", "-c:a", "copy", "-c:s", "copy"])
                    .args(if webm { ["-c:v", "libvpx-vp9", "-b:v", "0"] } else { ["-c:v", "libx264", "-preset", "medium"] })
                    // yuv420p needs even sizes
                    .args(["-crf", "18", "-pix_fmt", "yuv420p", "-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2"])
                    .arg(out);
                c
            }
            Sink::Player => {
                let mut c = command("ffplay");
                c.args(["-v", "error", "-f", "rawvideo", "-pixel_format", "rgb24", "-video_size", &size, "-framerate", &fps])
                    .args(["-window_title", "Pixlflow", "-autoexit", "-"]);
                c
            }
        };
        let mut child = spawn(c.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()))?;
        let stdin = child.stdin.take();
        let log = Log::take(&mut child);
        Ok(Output { child, stdin, log })
    }

    /// False once the other side stopped reading (e.g. the player window was closed).
    fn write(&mut self, px: &[Rgb]) -> bool {
        self.stdin.as_mut().is_some_and(|s| s.write_all(px.as_flattened()).is_ok())
    }

    fn finish(mut self) -> Result<(), String> {
        drop(self.stdin.take()); // end of input: ffmpeg writes the file's trailer
        let status = self.child.wait().map_err(|e| e.to_string())?;
        if status.success() { Ok(()) } else { Err(format!("FFmpeg failed: {}", self.log.text())) }
    }
}

/// Sends every frame of `input` through `sort` into `sink`. `go(frame)` is asked before each frame; `false`
/// stops early, and a file made so far is still closed properly. Returns the number of frames written.
pub fn run(
    input: &Path,
    info: &Info,
    sink: &Sink,
    mut sort: impl FnMut(usize, Vec<Rgb>) -> Vec<Rgb>,
    mut go: impl FnMut(usize) -> bool,
) -> Result<usize, String> {
    let mut frames = Frames::open(input, info, 0.0)?;
    let mut out = Output::open(sink, info)?;
    let mut n = 0;
    let mut closed = false;
    while go(n) {
        let Some(f) = frames.next_frame() else { break };
        if !out.write(&sort(n, f)) {
            closed = true;
            break;
        }
        n += 1;
    }
    let decode_error = if n == 0 { frames.error() } else { String::new() };
    drop(frames);
    match (out.finish(), sink) {
        (Err(e), Sink::File { .. }) if !closed => Err(e),
        _ if n == 0 => Err(format!("no frames could be read from '{}': {decode_error}", input.display())),
        _ => Ok(n), // a closed player window is a normal way to stop
    }
}

/// One decoded frame at `t` seconds.
pub fn frame_at(path: &Path, info: &Info, t: f64) -> Result<Vec<Rgb>, String> {
    let mut f = Frames::open(path, info, t)?;
    match f.next_frame() {
        Some(px) => Ok(px),
        None => Err(format!("no frame at {t:.2} s in '{}': {}", path.display(), f.error())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs FFmpeg; skipped when it isn't installed (e.g. on CI).
    #[test]
    fn sorts_a_video_and_keeps_its_audio() {
        let dir = std::env::temp_dir().join(format!("pixlflow-video-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("in.mp4");
        let made = command("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=63x47:rate=10:duration=1", "-f", "lavfi"])
            .args(["-i", "sine=duration=1", "-c:v", "libx264", "-pix_fmt", "yuv444p", "-c:a", "aac", "-shortest"])
            .arg(&src)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("FFmpeg not available, video test skipped");
            return;
        }
        let info = probe(&src).unwrap();
        assert_eq!((info.w, info.h, info.frames), (63, 47, 10));
        assert!((info.fps - 10.0).abs() < 1e-6);

        let out = dir.join("out.mp4");
        let mut seen = Vec::new();
        let n = run(&src, &info, &Sink::File { out: out.clone(), streams_from: src.clone() }, |i, px| {
            seen.push(i);
            px
        }, |_| true)
        .unwrap();
        assert_eq!(n, 10);
        assert_eq!(seen, (0..10).collect::<Vec<_>>());
        let probed = command("ffprobe").args(["-v", "error", "-show_entries", "stream=codec_type", "-of", "csv=p=0"]).arg(&out).output().unwrap();
        let kinds = String::from_utf8_lossy(&probed.stdout);
        assert!(kinds.contains("video") && kinds.contains("audio"), "streams: {kinds}");
        assert_eq!(probe(&out).unwrap().w, 64, "odd width padded to even");

        // stopping early still leaves a playable file
        let n = run(&src, &info, &Sink::File { out: out.clone(), streams_from: src.clone() }, |_, px| px, |i| i < 4).unwrap();
        assert_eq!(n, 4);
        assert!(probe(&out).unwrap().frames >= 4);

        assert_eq!(frame_at(&src, &info, 0.5).unwrap().len(), 63 * 47);
        assert!(probe(&dir.join("missing.mp4")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
