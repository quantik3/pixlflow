//! Named presets in `pixlflow-presets.txt` next to the program (portable): one `name<TAB>preset text` per line.

use std::path::PathBuf;

fn file() -> Option<PathBuf> {
    std::env::current_exe().ok().map(|p| p.with_file_name("pixlflow-presets.txt"))
}

pub fn load() -> Vec<(String, String)> {
    let text = file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    text.lines().filter_map(|l| l.split_once('\t')).map(|(n, t)| (n.to_string(), t.to_string())).collect()
}

pub fn save(list: &[(String, String)]) -> Result<(), String> {
    let f = file().ok_or("cannot find the program folder")?;
    let text: String = list.iter().map(|(n, t)| format!("{n}\t{t}\n")).collect();
    std::fs::write(&f, text).map_err(|e| format!("cannot save presets to '{}': {e}", f.display()))
}

/// A name that fits on one line of the file.
pub fn clean_name(name: &str) -> String {
    name.replace(['\t', '\n', '\r'], " ").trim().to_string()
}
