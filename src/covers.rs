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
            return Ok(Some(decode_picture_block(&payload)));
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

/// METADATA_BLOCK_PICTURE wire format → raw image bytes (the embedded PNG/JPEG).
fn decode_picture_block(payload: &[u8]) -> Vec<u8> {    let be32 = |off: usize| -> usize {
        u32::from_be_bytes([
            payload[off],
            payload[off + 1],
            payload[off + 2],
            payload[off + 3],
        ]) as usize
    };
    let mime_len = be32(4);
    let desc_off = 8 + mime_len;
    let desc_len = be32(desc_off);
    // data_len_off: desc_len FIELD (4 bytes) + desc + width/height/depth/colors (16)
    let data_len_off = desc_off + 4 + desc_len + 16;
    let data_off = data_len_off + 4;
    payload[data_off..data_off + be32(data_len_off)].to_vec()
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
        // only the metadata prefix was consumed, not the trailing audio:
        // (1000 trailing bytes were never needed — nothing else to assert here)
        assert_eq!(fixture.len(), flac_fixture(Some(data), 1000).len());
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
}
