//! Library cache: list the phone's audio files, save and load the listing.
use std::io;
use std::path::{Path, PathBuf};

/// Audio extensions we recognize (same list as flacast).
pub const EXTENSIONS: &[&str] = &["flac", "m4a", "mp3", "ogg", "opus", "wav"];

/// Default cache location: $XDG_CACHE_HOME/flactui/library.list
pub fn cache_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("flactui")
        .join("library.list")
}

/// Walk `root` recursively and return audio file paths relative to `root`,
/// sorted case-insensitively. Any unreadable directory is an ERROR: a partial
/// listing must never be cached (it would persist until a manual rm).
pub fn build(root: &Path) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort_by_key(|a| a.to_lowercase());
    Ok(out)
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            walk(base, &path, out)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
                if let Ok(rel) = path.strip_prefix(base) {
                    out.push(rel.to_string_lossy().into_owned());
                }
            }
        }
    }
    Ok(())
}

/// Load the listing (one relative path per line). Lines that cannot be audio
/// files (e.g. phantom halves of a filename containing \n) are dropped.
pub fn load(path: &Path) -> io::Result<Vec<String>> {
    Ok(std::fs::read_to_string(path)?
        .lines()
        .filter(|l| {
            let ext = l.rsplit('.').next().unwrap_or("");
            !l.is_empty() && EXTENSIONS.contains(&ext.to_lowercase().as_str())
        })
        .map(String::from)
        .collect())
}

/// Save the listing atomically (write .tmp then rename).
pub fn save(path: &Path, tracks: &[String]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("list.tmp");
    std::fs::write(&tmp, tracks.join("\n") + if tracks.is_empty() { "" } else { "\n" })?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("flactui-test-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn build_finds_audio_only_relative_sorted() {
        let d = tmpdir("build");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.flac"), "").unwrap();
        std::fs::write(d.join("sub/b.m4a"), "").unwrap();
        std::fs::write(d.join("notes.txt"), "").unwrap();
        std::fs::write(d.join(".hidden"), "").unwrap();
        let got = build(&d).unwrap();
        assert_eq!(got, vec!["a.flac", "sub/b.m4a"]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn build_sorts_case_insensitively() {
        let d = tmpdir("sort");
        std::fs::write(d.join("Zebra.flac"), "").unwrap();
        std::fs::write(d.join("apple.flac"), "").unwrap();
        let got = build(&d).unwrap();
        // case-SENSITIVE byte order would put "Zebra" (0x5A) first;
        // case-insensitive puts "apple" first.
        assert_eq!(got, vec!["apple.flac", "Zebra.flac"]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_load_roundtrip() {
        let d = tmpdir("roundtrip");
        let p = d.join("flactui/library.list");
        let tracks = vec!["one.flac".into(), "two dir/two.flac".into()];
        save(&p, &tracks).unwrap();
        assert_eq!(load(&p).unwrap(), tracks);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn load_missing_file_is_err() {
        let d = tmpdir("missing");
        assert!(load(&d.join("nope.list")).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn empty_cache_saves_and_loads_empty() {
        let d = tmpdir("empty");
        let p = d.join("library.list");
        save(&p, &[]).unwrap();
        assert_eq!(load(&p).unwrap(), Vec::<String>::new());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn build_propagates_unreadable_dir() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmpdir("unreadable");
        let sub = d.join("locked");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(d.join("ok.flac"), "").unwrap();
        std::fs::set_permissions(&sub, std::os::unix::fs::PermissionsExt::from_mode(0o000))
            .unwrap();
        let res = build(&d);
        // restore so remove_dir_all works
        std::fs::set_permissions(&sub, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        assert!(res.is_err(), "partial listings must not be cached silently");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn load_drops_lines_without_audio_extension() {
        // a filename containing \n splits into phantom lines; load must drop
        // lines that cannot be audio files
        let d = tmpdir("phantom");
        let p = d.join("library.list");
        std::fs::write(&p, "real.flac\nx\nbroken\nmusic.mp3\n").unwrap();
        assert_eq!(load(&p).unwrap(), vec!["real.flac", "music.mp3"]);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
