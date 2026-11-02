//! Source text helpers and the directory walk.
//!
//! Paths are `/`-joined strings (`${srcDir}/scripts/...`), because several checks compare them as strings.
//! The walk is pre-order with each directory listed (as `dir/`) before its
//! contents, in **bytewise** name order (B.1).
//! No caching: the clean build reads every file once.

/// Strict integer syntax for content values: optional `-`, then decimal
/// digits or `0x`/`0X` hex digits. Parsed as `i64` so values past `i32` reach
/// the writers, which wrap them explicitly.
pub fn int(s: &str) -> Option<i64> {
    let (neg, s) = s.strip_prefix('-').map_or((false, s), |d| (true, d));
    let (radix, d) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).map_or((10, s), |h| (16, h));
    if d.is_empty() || !d.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let n = i64::from_str_radix(d, radix).ok()?;
    Some(if neg { -n } else { n })
}

/// [`int`], or a pack error naming where the text came from (`file:line`).
pub fn int_at(s: &str, at: impl std::fmt::Display) -> Result<i64, String> {
    int(s).ok_or_else(|| format!("{at}: bad number {s:?}"))
}

/// The text between the first and last character: `[name]` → `name` (`''` when shorter than 2).
pub fn inner(s: &str) -> &str {
    s.get(1..s.len().saturating_sub(1)).unwrap_or("")
}

/// Node `path.basename(p)` for `/`-separated paths.
pub fn basename(p: &str) -> &str {
    let p = p.trim_end_matches('/');
    p.rsplit_once('/').map_or(p, |(_, b)| b)
}

/// Node `path.basename(p, ext)`.
pub fn basename_ext<'a>(p: &'a str, ext: &str) -> &'a str {
    let b = basename(p);
    match b.strip_suffix(ext) {
        Some(s) if !s.is_empty() => s,
        _ => b,
    }
}

/// Node `path.dirname(p)` for `/`-separated paths.
pub fn dirname(p: &str) -> &str {
    let p = p.trim_end_matches('/');
    match p.rfind('/') {
        Some(0) => "/",
        Some(i) => &p[..i],
        None => ".",
    }
}

/// `FsCache.listDir`: every entry below `path`, directories as `path/name/`
/// followed by their contents, bytewise order. Missing directory → empty.
pub fn list_dir(path: &str) -> Vec<String> {
    let path = path.strip_suffix('/').unwrap_or(path);
    let Ok(rd) = std::fs::read_dir(path) else { return Vec::new() };
    let mut entries: Vec<(String, bool)> = rd
        .filter_map(Result::ok)
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.file_type().is_ok_and(|t| t.is_dir())))
        .collect();
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut all = Vec::new();
    for (name, is_dir) in entries {
        if is_dir {
            all.push(format!("{path}/{name}/"));
            all.extend(list_dir(&format!("{path}/{name}")));
        } else {
            all.push(format!("{path}/{name}"));
        }
    }
    all
}

/// `listFilesExt(path, ext)`.
pub fn list_files_ext(path: &str, ext: &str) -> Vec<String> {
    list_dir(path).into_iter().filter(|f| f.ends_with(ext)).collect()
}

/// `readTextFile(path)` (utf8): invalid sequences become U+FFFD, a leading BOM is dropped.
pub fn read_text(path: &str) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes)).into_owned())
}

/// `loadFile(path)`: the file split on `\r?\n`; missing file → empty.
pub fn load_file(path: &str) -> Vec<String> {
    match read_text(path) {
        Ok(t) => split_lines(&t),
        Err(_) => Vec::new(),
    }
}

/// `text.split(/\r?\n/)`.
pub fn split_lines(t: &str) -> Vec<String> {
    t.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect()
}

/// `loadFileFull(path)`: trims every line, drops empty lines, strips `//` and
/// (nested-counting) `/* */` comments.
pub fn load_file_full(path: &str) -> Result<Vec<String>, String> {
    let text = load_file(path);
    let mut lines = Vec::new();
    let mut multi_comment_start = 0;
    let mut multi_line_comments = 0;

    for (i, raw) in text.iter().enumerate() {
        let mut line = raw.trim().to_string();
        if multi_line_comments > 0 {
            while let Some(start) = line.find("/*") {
                line = line[start + 2..].trim_start().to_string();
                multi_line_comments += 1;
            }
            while multi_line_comments > 0
                && let Some(end) = line.find("*/")
            {
                line = line[end + 2..].trim_start().to_string();
                multi_line_comments -= 1;
            }
            if multi_line_comments > 0 {
                continue;
            }
        }

        if line.is_empty() {
            continue;
        }

        if let Some(c) = line.find("//") {
            line = line[..c].trim_end().to_string();
            if line.is_empty() {
                continue;
            }
        }

        if let Some(start) = line.find("/*") {
            match line.find("*/") {
                // Uses the first `*/` even when it precedes the `/*`: the two
                // slices then overlap (`*/f/*g*/` → `*/ff/*g*/`); deliberate.
                Some(end) => line = format!("{}{}", &line[..start], &line[end + 2..]),
                None => {
                    line.truncate(start);
                    multi_line_comments += 1;
                    if multi_comment_start == 0 {
                        multi_comment_start = i + 1;
                    }
                }
            }
            if line.is_empty() {
                continue;
            }
        }

        lines.push(line);
    }

    if multi_line_comments > 0 {
        return Err(format!("{path} has an unclosed multi-line comment! Line: {multi_comment_start}"));
    }
    Ok(lines)
}

/// `listFilesExt` + `loadFileFull` for each file (`loadDirExtFull`): `(lines, file)`,
/// read lazily so a read error surfaces only once the walk reaches that file.
pub fn load_dir_ext_full(path: &str, ext: &str) -> impl Iterator<Item = Result<(Vec<String>, String), String>> {
    list_files_ext(path, ext).into_iter().map(|file| Ok((load_file_full(&file)?, file)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_path_helpers() {
        assert_eq!(basename("a/b/c.loc"), "c.loc");
        assert_eq!(basename_ext("a/b/c.loc", ".loc"), "c");
        assert_eq!(basename_ext("a/.loc", ".loc"), ".loc");
        assert_eq!(dirname("a/b/c.loc"), "a/b");
        assert_eq!(dirname("c.loc"), ".");
        assert_eq!(basename(dirname(dirname("a/c"))), ".");
    }

    #[test]
    fn strict_int() {
        assert_eq!(int("-5"), Some(-5));
        assert_eq!(int("0x10"), Some(16));
        assert_eq!(int("0XfF"), Some(255));
        assert_eq!(int("4294967297"), Some(4294967297));
        for bad in ["12abc", "abc", "", "1.5", "-", "0x", "+1", " 1", "1e3", "0x-5"] {
            assert_eq!(int(bad), None, "{bad:?}");
        }
        assert_eq!(int_at("5x", "a.seq:3").unwrap_err(), "a.seq:3: bad number \"5x\"");
        assert_eq!((inner("[a]"), inner("[]"), inner("")), ("a", "", ""));
    }

    #[test]
    fn comments_are_stripped() {
        let dir = std::env::temp_dir().join(format!("ares-parse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.txt");
        std::fs::write(&f, "\u{FEFF}  a=1 // c\r\n/* x\ny */ b=2\n c /* d */ e\n\n*/f/*g*/\n").unwrap();
        let lines = load_file_full(f.to_str().unwrap()).unwrap();
        assert_eq!(lines, ["a=1", "b=2", "c  e", "*/ff/*g*/"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
