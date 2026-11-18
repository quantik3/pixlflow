//! Project files: every layer (settings, mask, guide, directions), the current layer and the image path.
//!
//! Own binary format: the line `PIXLFLOW-PROJECT 1`, then records `[tag: u8][length: u32 LE][bytes]`.
//! A layer record starts a layer; the buffer records after it belong to that layer. Unknown tags are skipped.

use crate::settings::Settings;

const MAGIC: &[u8] = b"PIXLFLOW-PROJECT 1\n";
const IMAGE: u8 = 1;
const CURRENT: u8 = 2;
const LAYER: u8 = 10; // preset text
const MASK: u8 = 11;
const GUIDE: u8 = 12;
const GUIDE_NAME: u8 = 13;
const DIRS: u8 = 14; // f32 LE

#[derive(Clone, Debug, Default)]
pub struct Project {
    pub image: String,
    pub current: usize,
    pub layers: Vec<ProjectLayer>,
}

#[derive(Clone, Debug, Default)]
pub struct ProjectLayer {
    pub settings: Settings,
    pub mask: Option<Vec<u8>>,
    pub guide: Option<Vec<u8>>,
    pub guide_name: Option<String>,
    pub dirs: Option<Vec<f32>>,
}

impl Project {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        let mut rec = |tag: u8, data: &[u8]| {
            out.push(tag);
            out.extend((data.len() as u32).to_le_bytes());
            out.extend(data);
        };
        rec(IMAGE, self.image.as_bytes());
        rec(CURRENT, &(self.current as u32).to_le_bytes());
        for l in &self.layers {
            rec(LAYER, l.settings.to_preset().as_bytes());
            if let Some(m) = &l.mask {
                rec(MASK, m);
            }
            if let Some(g) = &l.guide {
                rec(GUIDE, g);
            }
            if let Some(n) = &l.guide_name {
                rec(GUIDE_NAME, n.as_bytes());
            }
            if let Some(d) = &l.dirs {
                rec(DIRS, &d.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
            }
        }
        out
    }

    pub fn from_bytes(b: &[u8]) -> Result<Project, String> {
        let damaged = || "the project file is damaged".to_string();
        let mut rest = b.strip_prefix(MAGIC).ok_or("not a Pixlflow project file")?;
        let mut p = Project::default();
        while let [tag, tail @ ..] = rest {
            let len = u32::from_le_bytes(tail.get(..4).ok_or_else(damaged)?.try_into().unwrap()) as usize;
            let data = tail.get(4..4 + len).ok_or_else(damaged)?;
            rest = &tail[4 + len..];
            let text = || String::from_utf8(data.to_vec()).map_err(|_| damaged());
            if *tag == IMAGE {
                p.image = text()?;
            } else if *tag == CURRENT {
                p.current = u32::from_le_bytes(data.try_into().map_err(|_| damaged())?) as usize;
            } else if *tag == LAYER {
                p.layers.push(ProjectLayer { settings: Settings::from_preset(&text()?)?, ..Default::default() });
            } else if matches!(*tag, MASK | GUIDE | GUIDE_NAME | DIRS) {
                let l = p.layers.last_mut().ok_or_else(damaged)?;
                match *tag {
                    MASK => l.mask = Some(data.to_vec()),
                    GUIDE => l.guide = Some(data.to_vec()),
                    GUIDE_NAME => l.guide_name = Some(text()?),
                    _ => l.dirs = Some(data.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()),
                }
            } // other tags: written by a newer version, skipped
        }
        if p.layers.is_empty() {
            return Err(damaged());
        }
        p.current = p.current.min(p.layers.len() - 1);
        Ok(p)
    }

    /// Drops masks, guides and directions that don't match a w×h image.
    pub fn fit(&mut self, w: usize, h: usize) {
        for l in &mut self.layers {
            l.mask = l.mask.take().filter(|m| m.len() == w * h);
            l.guide = l.guide.take().filter(|g| g.len() == w * h);
            if l.guide.is_none() {
                l.guide_name = None;
            }
            l.dirs = l.dirs.take().filter(|d| d.len() == w * h);
        }
    }
}
