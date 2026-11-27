//! Album-cover extraction and half-block rendering.
//!
//! Reads the FLAC `METADATA_BLOCK_PICTURE` block from the phone's FUSE mount
//! (metadata region only — never the whole audio stream), decodes it with
//! `image` and turns it into a fixed-size grid of half-block cells that the
//! UI paints with truecolor.
use std::io::Read;

/// Parse a FLAC stream and return the first PICTURE (type 6) metadata block's
/// payload. `None` when the metadata section ends without a picture. Only the
/// metadata prefix is read; audio frames are never consumed.
pub fn flac_picture_bytes(r: &mut impl Read) -> anyhow::Result<Option<Vec<u8>>> {
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    if &magic != b"fLaC" {
        anyhow::bail!("not a FLAC stream");
    }
    loop {
        let mut header = [0u8; 4];
        if r.read_exact(&mut header).is_err() {
            // metadata section ended without a picture block
            return Ok(None);
        }
        let last = header[0] & 0x80 != 0;
        let block_type = header[0] & 0x7f;
        let len = u32::from_be_bytes([header[1], header[2], header[3], 0]) >> 8;
        if block_type == 6 {
            let mut payload = vec![0u8; len as usize];
            r.read_exact(&mut payload)?;
            let data = decode_picture_block(&payload)
                .ok_or_else(|| anyhow::anyhow!("malformed PICTURE block"))?;
            return Ok(Some(data));
        }
        let mut skip = vec![0u8; len as usize];
        r.read_exact(&mut skip)?;
        if last {
            return Ok(None);
        }
    }
}

/// Popup grid size in cells; each cell paints 2 vertical pixels (half-blocks).
pub const GRID_W: u16 = 48;
pub const GRID_H: u16 = 24;

/// One rendered cell: top pixel = foreground, bottom pixel = background.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HalfCell {
    pub top: [u8; 3],
    pub bottom: [u8; 3],
}

/// A decoded cover as a fixed-size row-major grid of half-block cells.
#[derive(Clone, Debug)]
pub struct HalfBlocks {
    pub w: u16,
    pub h: u16,
    pub cells: Vec<HalfCell>,
}

/// Decode an image (PNG/JPEG bytes) and resize it into the fixed grid of
/// half-block cells (top pixel = fg, bottom pixel = bg per cell).
pub fn half_blocks(bytes: &[u8]) -> anyhow::Result<HalfBlocks> {
    let img = image::load_from_memory(bytes)?;
    // each cell = 1 horizontal pixel, 2 vertical pixels (half-blocks)
    let px_w = GRID_W as u32;
    let px_h = GRID_H as u32 * 2;
    let resized = image::imageops::resize(&img, px_w, px_h, image::imageops::FilterType::Triangle);
    let rgb = image::DynamicImage::ImageRgba8(resized).to_rgb8();
    let mut cells = Vec::with_capacity((GRID_W as usize) * (GRID_H as usize));
    for cy in 0..GRID_H {
        for cx in 0..GRID_W {
            let px = |x: u32, y: u32| -> [u8; 3] {
                let p = rgb.get_pixel(x, y).0;
                [p[0], p[1], p[2]]
            };
            let x = cx as u32;
            let top = px(x, cy as u32 * 2);
            let bottom = px(x, cy as u32 * 2 + 1);
            cells.push(HalfCell { top, bottom });
        }
    }
    Ok(HalfBlocks { w: GRID_W, h: GRID_H, cells })
}

/// Decoding errors surfaced to the popup.
#[derive(Debug)]
pub enum CoverError {
    /// Valid FLAC, but the tags carry no picture.
    NoPicture,
    /// File unreadable, not a FLAC, or the picture failed to decode.
    Unavailable(String),
}
impl std::fmt::Display for CoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoverError::NoPicture => write!(f, "no embedded cover"),
            CoverError::Unavailable(e) => write!(f, "cover unavailable: {e}"),
        }
    }
}

/// `~/.cache/flactui/covers` by default.
pub fn default_cache_dir() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("flactui")
        .join("covers")
}

/// Stable per-track cache file: hex hash of the absolute track path.
pub fn cache_path(cache_dir: &std::path::Path, track_path: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    track_path.hash(&mut h);
    cache_dir.join(format!("{:016x}.img", h.finish()))
}

fn read_cached(cache_file: &std::path::Path) -> Option<Vec<u8>> {
    std::fs::read(cache_file).ok()
}

fn store_cached(cache_dir: &std::path::Path, track_path: &str, bytes: &[u8]) {
    let _ = std::fs::create_dir_all(cache_dir);
    let _ = std::fs::write(cache_path(cache_dir, track_path), bytes);
}

/// Extract + decode the cover of `track_path`, using `cache_dir` when given.
pub fn load_with_cache(
    track_path: &str,
    cache_dir: Option<&std::path::Path>,
) -> Result<HalfBlocks, CoverError> {
    if let Some(dir) = cache_dir {
        if let Some(bytes) = read_cached(&cache_path(dir, track_path)) {
            if let Ok(g) = half_blocks(&bytes) {
                return Ok(g);
            }
            // corrupt cache entry → fall through to re-extraction
        }
    }
    let picture = std::fs::File::open(track_path)
        .map_err(|e| CoverError::Unavailable(format!("open: {e}")))
        .and_then(|mut f| {
            flac_picture_bytes(&mut f)
                .map_err(|e| CoverError::Unavailable(e.to_string()))
                .and_then(|p| p.ok_or(CoverError::NoPicture))
        })?;
    if let Some(dir) = cache_dir {
        store_cached(dir, track_path, &picture);
    }
    half_blocks(&picture).map_err(|e| CoverError::Unavailable(e.to_string()))
}

/// Extract + decode using the default cache directory.
pub fn load(track_path: &str) -> Result<HalfBlocks, CoverError> {
    load_with_cache(track_path, Some(&default_cache_dir()))
}

/// Track display name from a full path.
pub fn track_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// METADATA_BLOCK_PICTURE wire format → raw image bytes (the embedded PNG/JPEG).
/// Every field comes from an untrusted file: all reads are bounds-checked and
/// any malformation yields `None` (the caller maps that to a decode error).
fn decode_picture_block(payload: &[u8]) -> Option<Vec<u8>> {
    let be32 = |off: usize| -> Option<u32> {
        let b = payload.get(off..off + 4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let mime_len = be32(4)? as usize;
    let desc_off = 8usize.checked_add(mime_len)?;
    let desc_len = be32(desc_off)? as usize;
    // desc_len FIELD (4 bytes) + desc + width/height/depth/colors (16)
    let data_len_off = desc_off.checked_add(4)?.checked_add(desc_len)?.checked_add(16)?;
    let data_len = be32(data_len_off)? as usize;
    let data_off = data_len_off.checked_add(4)?;
    let end = data_off.checked_add(data_len)?;
    payload.get(data_off..end).map(<[u8]>::to_vec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be32(v: u32) -> [u8; 4] {
        v.to_be_bytes()
    }

    fn be24(v: usize) -> [u8; 3] {
        [(v >> 16) as u8, (v >> 8) as u8, v as u8]
    }

    /// Build a minimal FLAC stream: magic + STREAMINFO + optional PICTURE
    /// block + trailing audio bytes.
    fn flac_fixture(picture: Option<&[u8]>, trailing: usize) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"fLaC");
        // STREAMINFO: last iff there is no PICTURE block after it
        v.push(if picture.is_some() { 0x00 } else { 0x80 });
        v.extend_from_slice(&be24(34));
        v.extend_from_slice(&[0u8; 34]);
        if let Some(data) = picture {
            let mut payload = Vec::new();
            payload.extend_from_slice(&be32(3)); // front cover
            payload.extend_from_slice(&be32(9));
            payload.extend_from_slice(b"image/png");
            payload.extend_from_slice(&be32(0)); // empty description
            for w in [0u32; 4] {
                payload.extend_from_slice(&be32(w)); // width/height/depth/colors
            }
            payload.extend_from_slice(&be32(data.len() as u32));
            payload.extend_from_slice(data);
            v.push(0x80 | 6); // last block, type PICTURE
            v.extend_from_slice(&be24(payload.len()));
            v.extend_from_slice(&payload);
        }
        v.extend(std::iter::repeat_n(0xaau8, trailing));
        v
    }

    #[test]
    fn flac_picture_extracts_type6_block() {
        let data = b"\x89PNG-fake-bytes";
        let fixture = flac_fixture(Some(data), 1000);
        let got = flac_picture_bytes(&mut fixture.as_slice()).unwrap();
        assert_eq!(got.as_deref(), Some(data.as_slice()));
    }

    #[test]
    fn flac_picture_none_when_absent() {
        assert!(flac_picture_bytes(&mut flac_fixture(None, 500).as_slice())
            .unwrap()
            .is_none());
    }

    #[test]
    fn flac_picture_truncated_block_errors() {
        let data = b"\x89PNG-fake-bytes";
        let mut v = flac_fixture(Some(data), 0);
        // lie about the payload length: 10k bytes that are not there
        let plen = 10_000usize;
        let off = 4 + 4 + 34 + 1; // magic + STREAMINFO header + body + PICTURE flag byte
        v[off..off + 3].copy_from_slice(&be24(plen));
        assert!(flac_picture_bytes(&mut v.as_slice()).is_err());
    }

    #[test]
    fn flac_picture_malformed_payload_errors_not_panics() {
        // hostile PICTURE payload fields (attacker = a corrupted file on the
        // phone): huge mime_len / desc_len / data_len must yield Err, never a
        // panic — the TUI must survive any file the FUSE mount serves
        let build = |mime_len: u32, desc_len: u32, data_len: u32| -> Vec<u8> {
            let mut payload = Vec::new();
            payload.extend_from_slice(&be32(3));
            payload.extend_from_slice(&be32(mime_len));
            payload.extend_from_slice(b"image/png");
            payload.extend_from_slice(&be32(desc_len));
            for w in [0u32; 4] {
                payload.extend_from_slice(&be32(w));
            }
            payload.extend_from_slice(&be32(data_len));
            payload.extend_from_slice(b"\x89PNG-data");
            let mut v = Vec::new();
            v.extend_from_slice(b"fLaC");
            v.push(0x00);
            v.extend_from_slice(&be24(34));
            v.extend_from_slice(&[0u8; 34]);
            v.push(0x80 | 6);
            v.extend_from_slice(&be24(payload.len()));
            v.extend_from_slice(&payload);
            v
        };
        for (mime, desc, dlen) in [
            (0xFFFF_FFFF, 0, 0),
            (9, 0xFFFF_FFFF, 0),
            (9, 0, 0xFFFF_FFFF),
        ] {
            let v = build(mime, desc, dlen);
            let r = flac_picture_bytes(&mut v.as_slice());
            assert!(r.is_err(), "malformed payload ({mime},{desc},{dlen}) must Err");
        }
    }

    #[test]
    fn flac_picture_rejects_bad_magic() {
        assert!(flac_picture_bytes(&mut b"notfLaC-something".as_slice()).is_err());
    }

    /// 16×16 PNG: left half red, right half blue.
    fn half_red_blue_png() -> Vec<u8> {
        use image::{ImageBuffer, ImageFormat, Rgb, RgbImage};
        let mut img: RgbImage = ImageBuffer::new(16, 16);
        for (x, _, px) in img.enumerate_pixels_mut() {
            *px = if x < 8 { Rgb([255, 0, 0]) } else { Rgb([0, 0, 255]) };
        }
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn half_blocks_size_is_grid() {
        let g = half_blocks(&half_red_blue_png()).unwrap();
        assert_eq!((g.w, g.h), (GRID_W, GRID_H));
        assert_eq!(g.cells.len(), (GRID_W as usize) * (GRID_H as usize));
    }

    #[test]
    fn half_blocks_colors_split() {
        let g = half_blocks(&half_red_blue_png()).unwrap();
        let at = |x: u16, y: u16| g.cells[(y as usize) * g.w as usize + x as usize].top;
        let red = at(2, 2);
        let blue = at(46, 2);
        assert!(red[0] > 200 && red[1] < 60 && red[2] < 60, "left ≈ red, got {red:?}");
        assert!(blue[2] > 200 && blue[0] < 60 && blue[1] < 60, "right ≈ blue, got {blue:?}");
    }

    #[test]
    fn half_blocks_rejects_garbage() {
        assert!(half_blocks(b"notanimage").is_err());
    }

    // ---- load / cache ----

    struct Counting<T> {
        inner: T,
        read_bytes: std::rc::Rc<std::cell::Cell<usize>>,
    }
    impl<T: Read> Read for Counting<T> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.read_bytes.set(self.read_bytes.get() + n);
            Ok(n)
        }
    }

    fn temp_cache_dir(tag: &str) -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!(
            "flactui-covers-{}-{tag}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_fixture(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let png = half_red_blue_png();
        let p = dir.join(name);
        std::fs::write(&p, flac_fixture(Some(&png), 200_000)).unwrap();
        p
    }

    #[test]
    fn load_extracts_and_caches() {
        let dir = temp_cache_dir("extract");
        let p = write_fixture(&dir, "song.flac");
        let cache = temp_cache_dir("cache1");
        let g = load_with_cache(p.to_str().unwrap(), Some(&cache)).unwrap();
        assert_eq!((g.w, g.h), (GRID_W, GRID_H));
        // a cache file now exists
        assert!(std::fs::read_dir(&cache).unwrap().next().is_some());
    }

    #[test]
    fn load_uses_cache_without_reopening() {
        let dir = temp_cache_dir("cachehit");
        let p = write_fixture(&dir, "song.flac");
        let cache = temp_cache_dir("cache2");
        load_with_cache(p.to_str().unwrap(), Some(&cache)).unwrap();
        // the FLAC is gone; the cache alone must satisfy a second load
        std::fs::remove_file(&p).unwrap();
        assert!(load_with_cache(p.to_str().unwrap(), Some(&cache)).is_ok());
    }

    #[test]
    fn load_no_picture() {
        let dir = temp_cache_dir("nopict");
        let p = dir.join("song.flac");
        std::fs::write(&p, flac_fixture(None, 10_000)).unwrap();
        let cache = temp_cache_dir("cache3");
        assert!(matches!(
            load_with_cache(p.to_str().unwrap(), Some(&cache)),
            Err(CoverError::NoPicture)
        ));
    }

    #[test]
    fn load_missing_file_unavailable() {
        let cache = temp_cache_dir("cache4");
        assert!(matches!(
            load_with_cache("/nonexistent/song.flac", Some(&cache)),
            Err(CoverError::Unavailable(_))
        ));
    }

    #[test]
    fn load_reads_only_metadata_prefix() {
        let dir = temp_cache_dir("prefix");
        let p = write_fixture(&dir, "song.flac"); // 200_000 trailing audio bytes
        let cache = temp_cache_dir("cache5");
        let read_bytes = std::rc::Rc::new(std::cell::Cell::new(0));
        let counter = read_bytes.clone();
        let mut counting = Counting {
            inner: std::fs::File::open(&p).unwrap(),
            read_bytes,
        };
        // exercise the same pipeline by parsing the counting reader directly
        let pic = flac_picture_bytes(&mut counting).unwrap().unwrap();
        let _ = half_blocks(&pic).unwrap();
        assert!(
            counter.get() < 4096 + 200,
            "metadata-only read expected, read {} of {}",
            counter.get(),
            200_000
        );
    }

    #[test]
    fn load_without_writable_cache_still_extracts() {
        let dir = temp_cache_dir("nocache");
        let p = write_fixture(&dir, "song.flac");
        // cache "dir" is a regular file → unwritable
        let not_a_dir = dir.join("blocker");
        std::fs::write(&not_a_dir, b"x").unwrap();
        assert!(load_with_cache(p.to_str().unwrap(), Some(&not_a_dir)).is_ok());
    }
}
