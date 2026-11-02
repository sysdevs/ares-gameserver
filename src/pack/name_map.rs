//! Readers that decode with Node's `'ascii'` encoding, which clears the high bit of every byte (B.11).

use crate::pack::parse::{int_at, list_dir, split_lines};

/// `readTextFile(path, 'ascii')`.
pub fn read_ascii(path: &str) -> std::io::Result<String> {
    Ok(std::fs::read(path)?.into_iter().map(|b| (b & 0x7f) as char).collect())
}

/// `loadOrder(path)`: the non-empty lines as ints; missing file → empty.
pub fn load_order(path: &str) -> Result<Vec<i64>, String> {
    let Ok(t) = read_ascii(path) else { return Ok(Vec::new()) };
    split_lines(&t).iter().enumerate().filter(|(_, l)| !l.is_empty()).map(|(i, l)| int_at(l, format_args!("{path}:{}", i + 1))).collect()
}

/// `(1-based line number, line)` pairs.
pub type NumberedLines = Vec<(usize, String)>;

/// `loadDir(path, ext, cb)`: `(non-empty (line number, line)s, file name,
/// directory)` per file, read lazily in walk order.
pub fn load_dir(path: &str, ext: &str) -> impl Iterator<Item = Result<(NumberedLines, String, String), String>> {
    list_dir(path).into_iter().filter(move |f| f.ends_with(ext)).map(|file| {
        let text = read_ascii(&file).map_err(|e| format!("{file}: {e}"))?;
        let lines = split_lines(&text).into_iter().enumerate().filter(|(_, l)| !l.is_empty()).map(|(i, l)| (i + 1, l)).collect();
        let slash = file.rfind('/').unwrap_or(0);
        Ok((lines, file[slash + 1..].to_string(), file[..slash].to_string()))
    })
}
