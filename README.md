# Pixlflow

[Русская версия](README.ru.md)

A pixel sorter for Windows: a window with live preview, plus a command line. The image is cut into
**paths** (rows, circles, spirals, flow lines, brush-painted directions…), **spans** are selected on
each path, and the pixels of each span are sorted by brightness, hue and other values — or processed
by an effect (stretch, gradient, glitch…). Works on images and video.

## Features

- **20 path kinds**: rows and columns, diagonals at any angle, rays, circles and ellipses, spirals
  (round, square, frame-shaped), Hilbert curve, waves, tiles, snake, star; flow lines along noise,
  curl noise, vortices, the image's own contours or painted directions.
- **Span selectors**: whole path, fixed and random length, threshold on any value (including
  wrap-around, e.g. hue 340–20 = red), edges, two conditions joined by AND/OR, smoothing while
  searching for spans.
- **13 sort values**: brightness, hue, saturation, R/G/B channels, warmth, CIE L* lightness, chroma,
  min/max channel, distance to a color, brightness of a second (guide) image. Thresholds use the
  first twelve: the guide is another image, not a pixel's color.
- **12 algorithms**: stable sort, comb, "broken" glitch sort, partial sort ("drip"), stretch, shift,
  gradient, average, mirror, steps, shuffle, debug colors.
- **Extras**: herringbone (Alternate), single-channel sorting, posterize, soft span edges, chromatic
  aberration.
- **Layers** with 10 blend modes and opacity; each layer has its own mask, guide and directions.
- **Brushes**: mask (radius, hardness), rectangle and lasso for the mask (with a soft edge), and directions; masks from a threshold, a guide or a file.
- **Presets** — a layer's settings as one line of text; a named list, Random.
- **Animation** of a layer between two states → GIF or PNG frames.
- **Projects** (`.pxlf`): all layers, masks and directions together with the source path.
- **Video**: every frame goes through the whole layer stack, audio is copied without re-encoding;
  live playback.
- **Transparency** is kept: the alpha channel is not sorted and is written back.

## Installation

Ready build: download the zip from the Releases page and unpack it anywhere. It is the `pixlflow-windows`
artifact of the **CI** workflow (run it manually or push to `main`), with the license notices
(see "Third-party code").

Building from source (Windows):

1. [Rust](https://rustup.rs) with the **MSVC** toolchain and Visual Studio Build Tools (C++ component).
   The GNU toolchain does not work: it fails to build the windowing libraries.
2. `cargo build --release` → `target\release\pixlflow.exe`.

**Video** needs FFmpeg, which is not included: download a Windows build from the links on
[ffmpeg.org](https://ffmpeg.org/download.html) (gyan.dev or BtbN) and put `ffmpeg.exe`, `ffprobe.exe`
and `ffplay.exe` next to `pixlflow.exe` or in `PATH`. Everything else works without them.

## Window

`pixlflow` without arguments opens the window. Settings on the left, result on the right, status
line at the bottom. Changes are sorted right away in the background; while a slider is being dragged,
on a smaller copy.

| Keys and mouse | Action |
|---|---|
| Ctrl+O | open an image, video or project (or drop a file onto the window) |
| Ctrl+S | save to the output folder; the name describes the settings, existing files are never overwritten |
| Ctrl+Shift+S | save as… |
| Ctrl+Z / Ctrl+Y, Ctrl+Shift+Z | undo / redo (40 steps, all layers, masks and directions) |
| B | brush on/off |
| M | show the threshold mask |
| F, double click | fit the image to the window |
| wheel / middle button | zoom at cursor / pan |
| brush: left / right button | paint / erase (swapped with the Erase checkbox) |
| Rect / Lasso: drag | fill the mask in a rectangle / a drawn outline (right button erases) |

**Compare** shows source and result with a draggable divider. **Pick on image** sets a path's
center or a color by clicking the image.

**Layers**: "+ Add" adds a layer above the current one, "X" or a middle click deletes it. Each layer
is applied to the result of the layers below; after a change only that layer and the ones above it
are recomputed. The window shows the whole stack; "Up to this layer" shows the selected layer's result
without the layers above it (saving still uses all layers).

**Video** opens on its first frame, which is edited like an image. "Frame at" picks another frame,
"Play sorted" shows the sorted video in a player window, "Export video…" writes a file with audio.
With "Animate this layer over the clip" checked, the current layer moves from Start to End
(Animation section) over the length of the clip.

## Command line

```
pixlflow [OPTIONS] INPUT [OUTPUT]
pixlflow --gui [OPTIONS] [INPUT [OUTPUT]]
```

`INPUT`/`OUTPUT` are files or `-` for stdin/stdout (stdout gets PNG). Without `OUTPUT` the result is
saved as `<name>-sorted.<ext>` next to the input. Full option list: `pixlflow --help`.

```bash
# circles, wrap-around hue threshold — only red spans are sorted
pixlflow photo.jpg out.png --path circles --crit hue --min 340 --max 20

# a preset plus overrides on top of it
pixlflow --preset "path=flow algo=mirror select=full" --seed 7 photo.png

# pipeline
cat in.png | pixlflow - - --path diagonal --angle 60 > out.png

# a mask (same size) and a guide image
pixlflow photo.png --mask mask.png --guide depth.png --by guide

# video: whole file, playback, one frame
pixlflow clip.mp4 --path waves --amplitude 30
pixlflow clip.mp4 --play --algo glitch
pixlflow clip.mp4 --frame-at 12.5 --path spiral

# open the window with these settings; Save writes exactly to out.png
pixlflow --gui --path rays photo.png out.png
```

Exit codes: `0` success, `1` error while running, `2` invalid arguments. `-q` silences messages.

The release build is a windowed program (no console window). PowerShell and cmd don't wait for it
when it is started directly, so there use `Start-Process -Wait -NoNewWindow pixlflow …`; in Git Bash
and scripts output and exit codes arrive as usual.

## Presets

A preset is a line of `key=value` pairs separated by spaces; the keys are the command-line options:

```
path=circles cx=0.5 cy=0.4 select=threshold crit=hue min=340 max=20 by=brightness algo=sort blend=screen opacity=0.8
```

Unknown keys are ignored, missing ones take their defaults. Presets saved in the window live in
`pixlflow-presets.txt` next to the program (one per line: name, tab, text).

## Development

```bash
cargo test
```

Tests run in the debug build (integer overflow is an error there). The video test runs only when
FFmpeg is available and is skipped otherwise.

| Module | Contents |
|---|---|
| `src/settings.rs` | layer settings and the preset format |
| `src/color.rs`, `paths.rs`, `select.rs`, `algo.rs` | values, paths, selectors, algorithms |
| `src/engine.rs` | pipeline: paths → spans → algorithm → effects → blending |
| `src/brush.rs`, `anim.rs`, `project.rs`, `video.rs`, `imageio.rs` | brushes, animation, projects, video, files |
| `src/gui/` | the window (eframe/egui) |
| `src/main.rs` | command line |

## Third-party code

`THIRD_PARTY_NOTICES.txt` collects the notices; the CI build puts it next to the exe.

- The Hilbert curve for rectangles of any size is the "gilbert" algorithm by Jakub Červený,
  BSD-2-Clause; its notice is in that file.
- The Rust libraries compiled into `pixlflow.exe` (egui, image, rayon… and the fonts built into egui)
  are under MIT, Apache-2.0, OFL and similar licenses. CI collects their texts with
  [cargo-about](https://github.com/EmbarkStudios/cargo-about) into `CRATE-LICENSES.txt` in the
  artifact (config: `about.toml`, `about.hbs`). Locally: `cargo about generate about.hbs -o CRATE-LICENSES.txt`.
- FFmpeg is neither in the sources nor in the build: Pixlflow only runs the `ffmpeg` programs the
  user installed, as separate processes.

## License

Pixlflow is licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option.
