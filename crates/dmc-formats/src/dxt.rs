//! BC1 (DXT1) and BC3 (DXT5) block decoding to RGBA8, per the public
//! Direct3D block-compression specification. Blocks are always little-endian.

fn rgb565(c: u16) -> [u8; 3] {
    let r = (c >> 11) & 0x1F;
    let g = (c >> 5) & 0x3F;
    let b = c & 0x1F;
    [
        ((r << 3) | (r >> 2)) as u8,
        ((g << 2) | (g >> 4)) as u8,
        ((b << 3) | (b >> 2)) as u8,
    ]
}

fn mix(a: [u8; 3], b: [u8; 3], wa: u16, wb: u16) -> [u8; 3] {
    let d = wa + wb;
    std::array::from_fn(|i| ((a[i] as u16 * wa + b[i] as u16 * wb) / d) as u8)
}

/// Decode the 8-byte colour half of a block into 16 RGBA texels.
/// `four_colour` forces the opaque 4-colour mode (always the case in BC3).
fn colour_block(b: &[u8], four_colour: bool, out: &mut [[u8; 4]; 16]) {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let palette: [[u8; 4]; 4] = if four_colour || c0 > c1 {
        let (p2, p3) = (mix(p0, p1, 2, 1), mix(p0, p1, 1, 2));
        [rgba(p0, 255), rgba(p1, 255), rgba(p2, 255), rgba(p3, 255)]
    } else {
        [
            rgba(p0, 255),
            rgba(p1, 255),
            rgba(mix(p0, p1, 1, 1), 255),
            [0, 0, 0, 0],
        ]
    };
    for (i, texel) in out.iter_mut().enumerate() {
        *texel = palette[((bits >> (2 * i)) & 3) as usize];
    }
}

fn rgba(c: [u8; 3], a: u8) -> [u8; 4] {
    [c[0], c[1], c[2], a]
}

fn alpha_block(b: &[u8], out: &mut [[u8; 4]; 16]) {
    let (a0, a1) = (b[0] as u16, b[1] as u16);
    let mut palette = [0u8; 8];
    palette[0] = a0 as u8;
    palette[1] = a1 as u8;
    if a0 > a1 {
        for k in 1..7u16 {
            palette[k as usize + 1] = (((7 - k) * a0 + k * a1) / 7) as u8;
        }
    } else {
        for k in 1..5u16 {
            palette[k as usize + 1] = (((5 - k) * a0 + k * a1) / 5) as u8;
        }
        palette[6] = 0;
        palette[7] = 255;
    }
    let mut bits = 0u64;
    for (i, &byte) in b[2..8].iter().enumerate() {
        bits |= (byte as u64) << (8 * i);
    }
    for (i, texel) in out.iter_mut().enumerate() {
        texel[3] = palette[((bits >> (3 * i)) & 7) as usize];
    }
}

pub fn bc1_size(w: u32, h: u32) -> usize {
    (w.div_ceil(4).max(1) * h.div_ceil(4).max(1)) as usize * 8
}

pub fn bc3_size(w: u32, h: u32) -> usize {
    bc1_size(w, h) * 2
}

/// Decode a BC1 or BC3 surface. Missing trailing blocks decode as transparent
/// black rather than failing, so a truncated file still gives a usable preview.
pub fn decode(data: &[u8], w: u32, h: u32, bc3: bool) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    let step = if bc3 { 16 } else { 8 };
    let bw = w.div_ceil(4).max(1);
    let bh = h.div_ceil(4).max(1);
    let mut texels = [[0u8; 4]; 16];
    for by in 0..bh {
        for bx in 0..bw {
            let o = (by * bw + bx) * step;
            let Some(block) = data.get(o..o + step) else {
                return out;
            };
            if bc3 {
                colour_block(&block[8..], true, &mut texels);
                alpha_block(&block[..8], &mut texels);
            } else {
                colour_block(block, false, &mut texels);
            }
            for py in 0..4 {
                let y = by * 4 + py;
                if y >= h {
                    break;
                }
                for px in 0..4 {
                    let x = bx * 4 + px;
                    if x >= w {
                        break;
                    }
                    let p = (y * w + x) * 4;
                    out[p..p + 4].copy_from_slice(&texels[py * 4 + px]);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bc1_solid_red() {
        // c0 = pure red (0xF800), c1 = black, all indices 0.
        let block = [0x00, 0xF8, 0x00, 0x00, 0, 0, 0, 0];
        let px = decode(&block, 4, 4, false);
        assert!(px.chunks(4).all(|p| p == [255, 0, 0, 255]));
    }

    #[test]
    fn bc1_punch_through_alpha() {
        // c0 <= c1 selects 3-colour mode; index 3 is transparent.
        let block = [0x00, 0x00, 0x1F, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
        let px = decode(&block, 4, 4, false);
        assert!(px.chunks(4).all(|p| p == [0, 0, 0, 0]));
    }

    #[test]
    fn bc3_alpha_ramp_endpoints() {
        let mut block = [0u8; 16];
        block[0] = 200; // a0
        block[1] = 100; // a1; indices all 0 -> alpha 200
        block[8..10].copy_from_slice(&0x07E0u16.to_le_bytes()); // green
        let px = decode(&block, 4, 4, true);
        assert!(px.chunks(4).all(|p| p == [0, 255, 0, 200]));
    }

    #[test]
    fn non_multiple_of_four_and_truncated() {
        let px = decode(&[0x00, 0xF8, 0, 0, 0, 0, 0, 0], 5, 3, false);
        assert_eq!(px.len(), 5 * 3 * 4);
        assert_eq!(&px[0..4], &[255, 0, 0, 255]);
        // Second block column missing: stays zeroed.
        assert_eq!(&px[16..20], &[0, 0, 0, 0]);
    }
}
