//! Pixlflow command line.

use image::{imageops::FilterType, ColorType, DynamicImage, ImageFormat, RgbImage, RgbaImage};
use pixlflow::{color, engine, settings::Settings, Rgb};
use std::{
    io::{Cursor, Read, Write},
    path::Path,
    process::ExitCode,
    time::Instant,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Default)]
struct Args {
    settings: Settings,
    input: Option<String>,
    output: Option<String>,
    mask: Option<String>,
    guide: Option<String>,
    show_mask: bool,
    quiet: bool,
    gui: bool,
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        // shortcut: no GUI until stage 2, so no arguments shows help instead
        print_help();
        return ExitCode::SUCCESS;
    }
    match parse(&argv) {
        Ok(None) => ExitCode::SUCCESS,
        Ok(Some(a)) => match run(a) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("pixlflow: error: {e}");
                ExitCode::from(1)
            }
        },
        Err(e) => {
            eprintln!("pixlflow: {e}\nTry 'pixlflow --help'.");
            ExitCode::from(2)
        }
    }
}

/// `Ok(None)` when help/version was printed.
fn parse(argv: &[String]) -> Result<Option<Args>, String> {
    let mut a = Args::default();
    let mut positional = Vec::new();
    let mut it = argv.iter();
    while let Some(arg) = it.next() {
        if arg == "-" || !arg.starts_with('-') {
            positional.push(arg.clone());
            continue;
        }
        if arg == "--" {
            positional.extend(it.by_ref().cloned());
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
            "mask" => a.mask = Some(value()?),
            "guide" => a.guide = Some(value()?),
            "preset" => a.settings.apply_preset(&value()?)?,
            _ => {
                let info = Settings::keys().into_iter().find(|i| i.key == key).ok_or(format!("unknown option '{arg}'"))?;
                let v = if info.flag { inline.clone().unwrap_or_else(|| "true".into()) } else { value()? };
                a.settings.set(key, &v)?;
            }
        }
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
    if a.gui {
        return Err("the GUI is not available yet; give INPUT [OUTPUT] to sort from the command line".into());
    }
    let input = a.input.unwrap_or_default();
    let img = read_image(&input)?;
    if !matches!(img.color(), ColorType::L8 | ColorType::La8 | ColorType::Rgb8 | ColorType::Rgba8) && !a.quiet {
        eprintln!("pixlflow: warning: '{input}' has more than 8 bits per channel; reducing to 8 bits");
    }
    let (w, h) = (img.width() as usize, img.height() as usize);
    let alpha = img.color().has_alpha().then(|| img.to_rgba8().pixels().map(|p| p[3]).collect::<Vec<u8>>());
    let rgb: Vec<Rgb> = img.to_rgb8().pixels().map(|p| p.0).collect();

    let mask = a.mask.as_deref().map(|p| load_gray(p, w, h, false)).transpose()?;
    let guide = a.guide.as_deref().map(|p| load_gray(p, w, h, true)).transpose()?;

    let started = Instant::now();
    let (out, alpha) = if a.show_mask {
        (engine::selection_map(&a.settings, w, h, &rgb).into_iter().map(|v| [v; 3]).collect(), None)
    } else {
        let ex = engine::Extras { mask: mask.as_deref(), guide: guide.as_deref(), directions: None };
        (engine::render(&a.settings, w, h, &rgb, &ex), alpha)
    };
    let elapsed = started.elapsed();

    let output = a.output.unwrap_or_else(|| default_output(&input));
    save(&output, w as u32, h as u32, &out, alpha.as_deref())?;
    if !a.quiet {
        eprintln!("pixlflow: {w}x{h} in {} ms -> {output}", elapsed.as_millis());
    }
    Ok(())
}

fn read_image(path: &str) -> Result<DynamicImage, String> {
    let decoded = if path == "-" {
        let mut buf = Vec::new();
        std::io::stdin().read_to_end(&mut buf).map_err(|e| format!("cannot read stdin: {e}"))?;
        image::load_from_memory(&buf)
    } else {
        image::ImageReader::open(path)
            .and_then(|r| r.with_guessed_format())
            .map_err(|e| format!("cannot open '{path}': {e}"))?
            .decode()
    };
    decoded.map_err(|e| format!("cannot read image '{path}': {e}"))
}

/// Brightness of a grey image. A mask must match the image size; a guide is resized (bilinear).
fn load_gray(path: &str, w: usize, h: usize, resize: bool) -> Result<Vec<u8>, String> {
    let mut img = read_image(path)?;
    if (img.width() as usize, img.height() as usize) != (w, h) {
        if !resize {
            return Err(format!("mask '{path}' is {}x{}, but the image is {w}x{h}", img.width(), img.height()));
        }
        img = img.resize_exact(w as u32, h as u32, FilterType::Triangle);
    }
    Ok(img.to_rgb8().pixels().map(|p| color::brightness(p.0) as u8).collect())
}

fn default_output(input: &str) -> String {
    if input == "-" {
        return "-".into();
    }
    let p = Path::new(input);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("png");
    p.with_file_name(format!("{stem}-sorted.{ext}")).to_string_lossy().into_owned()
}

fn save(path: &str, w: u32, h: u32, px: &[Rgb], alpha: Option<&[u8]>) -> Result<(), String> {
    let fmt = if path == "-" {
        ImageFormat::Png
    } else {
        ImageFormat::from_path(path).map_err(|_| format!("unknown output format for '{path}' (use .png, .jpg, .webp or .gif)"))?
    };
    let img = match alpha {
        // the alpha channel never moves; it is written back where the format has one
        Some(a) if fmt != ImageFormat::Jpeg => DynamicImage::ImageRgba8(
            RgbaImage::from_raw(w, h, px.iter().zip(a).flat_map(|(p, &a)| [p[0], p[1], p[2], a]).collect()).unwrap(),
        ),
        _ => DynamicImage::ImageRgb8(RgbImage::from_raw(w, h, px.concat()).unwrap()),
    };
    let img = if fmt == ImageFormat::Gif { DynamicImage::ImageRgba8(img.to_rgba8()) } else { img };
    if path == "-" {
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, fmt).map_err(|e| format!("cannot encode PNG: {e}"))?;
        std::io::stdout().write_all(buf.get_ref()).map_err(|e| format!("cannot write stdout: {e}"))
    } else {
        img.save_with_format(path, fmt).map_err(|e| format!("cannot save '{path}': {e}"))
    }
}

fn print_help() {
    println!(
        "pixlflow {VERSION} - pixel sorter

Usage: pixlflow [OPTIONS] INPUT [OUTPUT]

INPUT/OUTPUT are image files or '-' for stdin/stdout (stdout is PNG).
Without OUTPUT the result goes to <name>-sorted.<ext> next to INPUT.

Options:
  --preset TEXT        start from preset text (\"key=value ...\"); later options override it
  --mask FILE          grey mask, same size as INPUT: black = untouched, white = full effect
  --guide FILE         guide image for '--by guide' (resized to INPUT)
  --show-mask          write the threshold selection map instead of sorting
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
