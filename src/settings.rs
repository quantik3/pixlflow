//! All per-layer settings and their text (preset) form.
//!
//! Preset format: whitespace-separated `key=value` tokens, e.g. `path=circles select=threshold min=40`.
//! The same keys are the command-line options (`--path circles`). Unknown keys are ignored when reading,
//! missing keys keep their defaults.

use crate::Rgb;

/// A setting value that can be written to and read from preset text.
pub trait Value: Sized {
    /// Boolean switches may be given on the command line without a value.
    const FLAG: bool = false;
    fn to_text(&self) -> String;
    fn from_text(s: &str) -> Option<Self>;
    fn choices() -> Option<&'static [&'static str]> {
        None
    }
}

macro_rules! num_value {
    ($($t:ty),*) => {$(
        impl Value for $t {
            fn to_text(&self) -> String { self.to_string() }
            fn from_text(s: &str) -> Option<Self> { s.parse().ok() }
        }
    )*};
}
num_value!(u32, i32, u64);

impl Value for f32 {
    fn to_text(&self) -> String {
        self.to_string() // shortest text that parses back to the same f32
    }
    fn from_text(s: &str) -> Option<Self> {
        s.parse().ok().filter(|v: &f32| v.is_finite())
    }
}

impl Value for bool {
    const FLAG: bool = true;
    fn to_text(&self) -> String {
        self.to_string()
    }
    fn from_text(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        }
    }
}

impl Value for Rgb {
    fn to_text(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self[0], self[1], self[2])
    }
    fn from_text(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#').unwrap_or(s);
        if s.len() != 6 || !s.is_ascii() {
            return None;
        }
        let ch = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
        Some([ch(0)?, ch(2)?, ch(4)?])
    }
}

macro_rules! named_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($var),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),+];
            pub fn name(self) -> &'static str { match self { $($name::$var => $s),+ } }
            pub fn from_name(s: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|v| v.name().eq_ignore_ascii_case(s))
            }
        }
        /// The first variant is the default.
        impl Default for $name { fn default() -> Self { Self::ALL[0] } }
        impl Value for $name {
            fn to_text(&self) -> String { self.name().to_string() }
            fn from_text(s: &str) -> Option<Self> { Self::from_name(s) }
            fn choices() -> Option<&'static [&'static str]> { Some(&[$($s),+]) }
        }
    };
}

named_enum!(PathKind {
    Lines = "lines",
    Columns = "columns",
    AllHorizontal = "all-h",
    AllVertical = "all-v",
    Diagonal = "diagonal",
    Rays = "rays",
    Circles = "circles",
    Spiral = "spiral",
    SquareSpiral = "square-spiral",
    RectSpiral = "rect-spiral",
    Hilbert = "hilbert",
    Waves = "waves",
    Flow = "flow",
    Curl = "curl",
    Swirls = "swirls",
    Contours = "contours",
    Drawn = "drawn",
    Star = "star",
    Tiles = "tiles",
    Snake = "snake",
});

named_enum!(SelectorKind {
    Threshold = "threshold",
    Full = "full",
    Fixed = "fixed",
    Random = "random",
    Edges = "edges",
    Combined = "combined",
});

named_enum!(Criterion {
    Brightness = "brightness",
    Hue = "hue",
    Saturation = "saturation",
    Red = "red",
    Green = "green",
    Blue = "blue",
    Warmth = "warmth",
    Lightness = "lightness",
    Chroma = "chroma",
    MinChannel = "min-channel",
    MaxChannel = "max-channel",
    ColorDistance = "distance",
    Guide = "guide",
});

named_enum!(Algorithm {
    Sort = "sort",
    Comb = "comb",
    Glitch = "glitch",
    Partial = "partial",
    Stretch = "stretch",
    Shift = "shift",
    Gradient = "gradient",
    Average = "average",
    Mirror = "mirror",
    Bucket = "bucket",
    Shuffle = "shuffle",
    Debug = "debug",
});

named_enum!(Channel { All = "all", Red = "r", Green = "g", Blue = "b" });

named_enum!(Combine { And = "and", Or = "or" });

pub struct KeyInfo {
    pub key: &'static str,
    pub help: &'static str,
    pub flag: bool,
    pub choices: Option<&'static [&'static str]>,
}

macro_rules! settings {
    ($($field:ident: $ty:ty = $def:expr, $key:literal, $help:literal;)+) => {
        #[derive(Clone, Debug, PartialEq)]
        pub struct Settings { $(pub $field: $ty),+ }

        impl Default for Settings {
            fn default() -> Self { Settings { $($field: $def),+ } }
        }

        impl Settings {
            pub fn keys() -> Vec<KeyInfo> {
                vec![$(KeyInfo { key: $key, help: $help, flag: <$ty as Value>::FLAG, choices: <$ty as Value>::choices() }),+]
            }
            /// Sets one value by key. `Ok(false)` means the key is unknown.
            pub fn set(&mut self, key: &str, val: &str) -> Result<bool, String> {
                match key {
                    $($key => {
                        self.$field = <$ty as Value>::from_text(val)
                            .ok_or_else(|| format!("invalid value '{val}' for '{key}'"))?;
                        Ok(true)
                    })+
                    _ => Ok(false),
                }
            }
            pub fn to_preset(&self) -> String {
                [$(format!("{}={}", $key, self.$field.to_text())),+].join(" ")
            }
        }
    };
}

settings! {
    path: PathKind = PathKind::Lines, "path", "path shape";
    reverse: bool = false, "reverse", "reverse every path";
    cx: f32 = 0.5, "cx", "path center x, fraction of width";
    cy: f32 = 0.5, "cy", "path center y, fraction of height";
    rotation: f32 = 0.0, "rotation", "circles/spiral rotation, degrees";
    aspect: f32 = 1.0, "aspect", "circles/spiral width:height, 0.2-5";
    angle: f32 = 30.0, "angle", "diagonal angle, degrees (0 right, 90 up)";
    amplitude: f32 = 20.0, "amplitude", "waves amplitude, px";
    wavelength: f32 = 200.0, "wavelength", "waves wavelength, px";
    wave_vertical: bool = false, "vertical-waves", "waves run top to bottom";
    noise_size: f32 = 200.0, "noise-size", "flow/curl swirl size, px";
    swirls: u32 = 5, "swirls", "number of swirl centers";
    contour_smooth: u32 = 8, "contour-smooth", "contours smoothing radius, px";
    star_points: u32 = 5, "star-points", "number of star rays";
    star_inner: f32 = 0.5, "star-inner", "star inner radius, 0-1";
    tile: u32 = 64, "tile", "tile size, px";
    seed: u64 = 1, "seed", "random seed (noise, swirls, random lengths, shift, shuffle)";
    select: SelectorKind = SelectorKind::Threshold, "select", "span selector";
    length: u32 = 100, "length", "fixed span length (0 = no spans)";
    max_length: u32 = 200, "max-length", "random span maximum length";
    crit: Criterion = Criterion::Brightness, "crit", "threshold criterion";
    min: u32 = 60, "min", "threshold minimum (min > max wraps around)";
    max: u32 = 200, "max", "threshold maximum";
    crit2: Criterion = Criterion::Hue, "crit2", "combined: second criterion";
    min2: u32 = 0, "min2", "combined: second minimum";
    max2: u32 = 360, "max2", "combined: second maximum";
    combine: Combine = Combine::And, "combine", "combined: how the conditions join";
    edge: u32 = 100, "edge", "edges threshold, 0-765";
    smooth: u32 = 0, "smooth", "blur radius used to find spans (0 = off)";
    by: Criterion = Criterion::Brightness, "by", "sort criterion";
    algo: Algorithm = Algorithm::Sort, "algo", "span algorithm";
    passes: u32 = 20, "passes", "partial sort passes";
    strength: f32 = 0.5, "strength", "stretch/shift strength, 0-1";
    buckets: u32 = 8, "buckets", "bucket levels, 2-64";
    alternate: bool = false, "alternate", "reverse every second span";
    channel: Channel = Channel::All, "channel", "channel that moves";
    posterize: u32 = 0, "posterize", "levels per channel (0/1 = off)";
    soft_ends: u32 = 0, "soft-ends", "blend span ends over N px (0 = off)";
    fringes: i32 = 0, "fringes", "chromatic shift of red/blue, px";
    target: Rgb = [255, 0, 0], "target", "color for the distance criterion (#rrggbb)";
}

impl Settings {
    /// Applies preset text on top of the current values.
    pub fn apply_preset(&mut self, text: &str) -> Result<(), String> {
        for tok in text.split_whitespace() {
            let (k, v) = tok.split_once('=').ok_or_else(|| format!("expected key=value, got '{tok}'"))?;
            self.set(k, v)?; // unknown keys are ignored
        }
        Ok(())
    }

    pub fn from_preset(text: &str) -> Result<Settings, String> {
        let mut s = Settings::default();
        s.apply_preset(text)?;
        Ok(s)
    }

    /// Settings for the same look on an image scaled by `k`: lengths in pixels are scaled.
    pub fn scaled(&self, k: f32) -> Settings {
        // non-zero lengths stay at least 1 (max-length at least 2) so spans don't vanish
        let px = |v: u32, min: u32| if v == 0 { 0 } else { ((v as f32 * k).round() as u32).max(min) };
        Settings {
            length: px(self.length, 1),
            max_length: if self.max_length > 1 { px(self.max_length, 2) } else { self.max_length },
            tile: px(self.tile, 1),
            contour_smooth: px(self.contour_smooth, 0),
            smooth: px(self.smooth, 0),
            soft_ends: px(self.soft_ends, 0),
            fringes: (self.fringes as f32 * k).round() as i32,
            amplitude: self.amplitude * k,
            wavelength: self.wavelength * k,
            noise_size: self.noise_size * k,
            ..self.clone()
        }
    }
}
