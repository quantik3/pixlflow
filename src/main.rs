//! Pixlflow: GUI when started without arguments, command line otherwise.

// Release builds are GUI programs (no console window); output still reaches the terminal they start from.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod gui;

use pixlflow::{
    engine, imageio,
    settings::{Criterion, Settings},
    video,
};
use std::{path::PathBuf, process::ExitCode, time::Instant};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Default)]
struct Args {
    settings: Settings,
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    mask: Option<PathBuf>,
    guide: Option<PathBuf>,
    show_mask: bool,
    quiet: bool,
    gui: bool,
    play: bool,
    frame_at: Option<f64>,
}

fn main() -> ExitCode {
    attach_console();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = if argv.is_empty() {
        Args { gui: true, ..Default::default() }
    } else {
        match parse(&argv) {
            Ok(Some(a)) => a,
            Ok(None) => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("pixlflow: {e}\nTry 'pixlflow --help'.");
                return ExitCode::from(2);
            }
        }
    };
    let result = if args.gui { gui::run(args.settings, args.input, args.output) } else { run(args) };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pixlflow: error: {e}");
            ExitCode::from(1)
        }
    }
}

/// A GUI-subsystem program has no console; borrow the parent's so messages reach the terminal.
#[cfg(windows)]
fn attach_console() {
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: plain Win32 call; failure (no parent console, or one already attached) is harmless.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(not(windows))]
fn attach_console() {}

/// `Ok(None)` when help/version was printed.
fn parse(argv: &[String]) -> Result<Option<Args>, String> {
    let mut a = Args::default();
    let mut positional = Vec::new();
    let mut it = argv.iter();
    while let Some(arg) = it.next() {
        if arg == "-" || !arg.starts_with('-') {
            positional.push(PathBuf::from(arg));
            continue;
        }
        if arg == "--" {
            positional.extend(it.by_ref().map(PathBuf::from));
            break;
        }
        let body = arg.trim_start_matches('-');
        let (key, inline) = match body.split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (body, None),
        };
        let mut value = || inline.clone().or_else(|| it.next().cloned()).ok_or(format!("option '--{key}' needs a value"));
        match key {
            "h" | "help" => {
                print_help();
                return Ok(None);
            }
            "V" | "version" => {
                println!("pixlflow {VERSION}");
                return Ok(None);
            }
            "q" | "quiet" => a.quiet = true,
            "show-mask" => a.show_mask = true,
            "gui" => a.gui = true,
            "play" => a.play = true,
            "frame-at" => {
                let v = value()?;
                a.frame_at = Some(v.parse().ok().filter(|t: &f64| *t >= 0.0).ok_or(format!("invalid time '{v}' for '--frame-at'"))?);
            }
            "mask" => a.mask = Some(value()?.into()),
            "guide" => a.guide = Some(value()?.into()),
            "preset" => a.settings.apply_preset(&value()?)?,
            _ => {
                let info = Settings::keys().into_iter().find(|i| i.key == key).ok_or(format!("unknown option '{arg}'"))?;
                let v = if info.flag { inline.clone().unwrap_or_else(|| "true".into()) } else { value()? };
                a.settings.set(key, &v)?;
            }
        }
    }
    if [a.settings.crit, a.settings.crit2].contains(&Criterion::Guide) {
        return Err("'guide' can't be a threshold criterion (--crit, --crit2); use it to sort: --by guide".into());
    }
    let mut positional = positional.into_iter();
    a.input = positional.next();
    a.output = positional.next();
    if positional.next().is_some() {
        return Err("too many file arguments (expected INPUT [OUTPUT])".into());
    }
    if a.input.is_none() && !a.gui {
        return Err("missing input file".into());
    }
    Ok(Some(a))
}

fn run(a: Args) -> Result<(), String> {
    let input = a.input.clone().unwrap_or_default();
    if video::is_video(&input) {
        return run_video(a, &input);
    }
    let pic = imageio::load(&input)?;
    if pic.deep && !a.quiet {
        eprintln!("pixlflow: warning: '{}' has more than 8 bits per channel; reducing to 8 bits", input.display());
    }
    let (w, h) = (pic.w, pic.h);
    let mask = a.mask.as_deref().map(|p| imageio::load_gray(p, w, h, false)).transpose()?;
    let guide = a.guide.as_deref().map(|p| imageio::load_gray(p, w, h, true)).transpose()?;

    let started = Instant::now();
    let (out, alpha) = if a.show_mask {
        (engine::selection_map(&a.settings, w, h, &pic.rgb).into_iter().map(|v| [v; 3]).collect(), None)
    } else {
        let ex = engine::Extras { mask: mask.as_deref(), guide: guide.as_deref(), directions: None };
        (engine::render(&a.settings, w, h, &pic.rgb, &ex), pic.alpha.as_deref())
    };
    let elapsed = started.elapsed();

    let output = a.output.unwrap_or_else(|| default_output(&input));
    imageio::save(&output, w, h, &out, alpha)?;
    if !a.quiet {
        eprintln!("pixlflow: {w}x{h} in {} ms -> {}", elapsed.as_millis(), output.display());
    }
    Ok(())
}

/// Sorts a whole video (or plays it sorted, or sorts one frame of it).
fn run_video(a: Args, input: &std::path::Path) -> Result<(), String> {
    let info = video::probe(input)?;
    let (w, h) = (info.w, info.h);
    let mask = a.mask.as_deref().map(|p| imageio::load_gray(p, w, h, false)).transpose()?;
    let guide = a.guide.as_deref().map(|p| imageio::load_gray(p, w, h, true)).transpose()?;
    let ex = engine::Extras { mask: mask.as_deref(), guide: guide.as_deref(), directions: None };
    let sort = |px: Vec<pixlflow::Rgb>| {
        if a.show_mask {
            engine::selection_map(&a.settings, w, h, &px).into_iter().map(|v| [v; 3]).collect()
        } else {
            engine::render(&a.settings, w, h, &px, &ex)
        }
    };
    if let Some(t) = a.frame_at {
        let out = a.output.unwrap_or_else(|| {
            let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            input.with_file_name(format!("{stem}-{t}s-sorted.png"))
        });
        imageio::save(&out, w, h, &sort(video::frame_at(input, &info, t)?), None)?;
        if !a.quiet {
            eprintln!("pixlflow: frame at {t} s -> {}", out.display());
        }
        return Ok(());
    }
    let sink = if a.play {
        video::Sink::Player
    } else {
        let out = a.output.clone().unwrap_or_else(|| default_output(input));
        if out == std::path::Path::new("-") || input == out {
            return Err("give a video file name to write to (stdout and the input itself are not allowed)".into());
        }
        video::Sink::File { out, streams_from: input.to_path_buf() }
    };
    let started = Instant::now();
    let total = info.frames.max(1);
    let n = video::run(input, &info, &sink, |_, px| sort(px), |i| {
        if !a.quiet && i % 5 == 0 {
            eprint!("\rpixlflow: frame {i} / {total} ({:.1} s)   ", i as f64 / info.fps);
        }
        true
    })?;
    if !a.quiet {
        let place = match &sink {
            video::Sink::File { out, .. } => out.display().to_string(),
            video::Sink::Player => "player".into(),
        };
        eprintln!("\rpixlflow: {n} frames ({w}x{h}) in {:.1} s -> {place}", started.elapsed().as_secs_f64());
    }
    Ok(())
}

/// `<name>-sorted.<ext>` next to the input (stdout for stdin).
pub fn default_output(input: &std::path::Path) -> PathBuf {
    if input == std::path::Path::new("-") {
        return input.into();
    }
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let ext = input.extension().and_then(|s| s.to_str()).unwrap_or("png");
    input.with_file_name(format!("{stem}-sorted.{ext}"))
}

fn print_help() {
    println!(
        "pixlflow {VERSION} - pixel sorter

Usage: pixlflow [OPTIONS] INPUT [OUTPUT]
       pixlflow --gui [OPTIONS] [INPUT [OUTPUT]]
       pixlflow                      (opens the window)

INPUT/OUTPUT are image files or '-' for stdin/stdout (stdout is PNG).
Without OUTPUT the result goes to <name>-sorted.<ext> next to INPUT.
With --gui the window opens with these settings; Save then writes exactly to OUTPUT if given.

Options:
  --gui                open the window instead of sorting once
  --preset TEXT        start from preset text (\"key=value ...\"); later options override it
  --mask FILE          grey mask, same size as INPUT: black = untouched, white = full effect
  --guide FILE         guide image for '--by guide' (resized to INPUT)
  --show-mask          write the threshold selection map instead of sorting

Video (INPUT .mp4 .mov .mkv .webm .avi .m4v; needs ffmpeg, ffprobe and ffplay next to
pixlflow.exe or in PATH). Every frame is sorted; audio and subtitles are copied as they are.
  --play               watch the sorted video instead of writing a file
  --frame-at SECONDS   sort only the frame at that time and write it as an image
  -q, --quiet          no messages
  -h, --help           this help
  -V, --version        version

Settings (--key VALUE or --key=VALUE; switches need no value):"
    );
    for k in Settings::keys() {
        let arg = if k.flag { String::new() } else { " V".into() };
        println!("  --{:<20} {}", format!("{}{arg}", k.key), k.help);
        if let Some(c) = k.choices {
            println!("  {:<22} {}", "", c.join(", "));
        }
    }
    println!(
        "
Examples:
  pixlflow photo.jpg out.png --path circles --select threshold --crit hue --min 340 --max 20
  pixlflow --preset \"path=flow algo=mirror\" --seed 7 photo.png
  cat in.png | pixlflow - - --path diagonal --angle 60 > out.png"
    );
}
