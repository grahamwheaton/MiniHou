//! Shared helpers for the time / physics nodes (goals, solver, rbd): Python-style parameter range checks and the topology
//! digest Python's `Topology.digest` computes (BLAKE2b, 16-byte digest).

use crate::error::{Error, Result};
use crate::geometry::Topology;
use crate::params::Params;

/// Python's `repr` of a number in an error message: integers without a fraction, floats like Python prints them.
fn show(v: f64, int: bool) -> String {
    if int { format!("{}", v as i64) } else { format!("{v:?}") }
}

/// A float parameter checked like `sdk.Param.coerce` (finite, within `min..=max` when given).
pub(super) fn float(p: &Params, name: &str, min: Option<f64>, max: Option<f64>) -> Result<f64> {
    let v = p.f(name)?;
    check(name, v, min, max, false)
}

/// An int parameter checked like `sdk.Param.coerce`.
pub(super) fn int(p: &Params, name: &str, min: Option<i64>, max: Option<i64>) -> Result<i64> {
    let v = p.i(name)?;
    check(name, v as f64, min.map(|x| x as f64), max.map(|x| x as f64), true)?;
    Ok(v)
}

/// A vec3 parameter: three finite numbers.
pub(super) fn vec3(p: &Params, name: &str) -> Result<[f64; 3]> {
    let v = p.v3(name)?;
    for c in v {
        check(name, c, None, None, false)?;
    }
    Ok(v)
}

fn check(name: &str, v: f64, min: Option<f64>, max: Option<f64>, int: bool) -> Result<f64> {
    if !v.is_finite() {
        return Err(Error::new(format!("parameter {name:?}: expected a finite number, got {v}")));
    }
    if let Some(m) = min.filter(|&m| v < m) {
        return Err(Error::new(format!("parameter {name:?}: {} is below minimum {}", show(v, int), show(m, int))));
    }
    if let Some(m) = max.filter(|&m| v > m) {
        return Err(Error::new(format!("parameter {name:?}: {} is above maximum {}", show(v, int), show(m, int))));
    }
    Ok(v)
}

// ---------------------------------------------------------------------------------------------------- BLAKE2b (RFC 7693)

const IV: [u64; 8] = [
    0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1,
    0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
];

const SIGMA: [[usize; 16]; 12] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
];

/// Streaming BLAKE2b without a key.
pub(super) struct Blake2b {
    h: [u64; 8],
    buf: [u8; 128],
    fill: usize,
    total: u128,
    out: usize,
}

impl Blake2b {
    pub(super) fn new(out: usize) -> Self {
        let mut h = IV;
        h[0] ^= 0x0101_0000 ^ out as u64;
        Blake2b { h, buf: [0; 128], fill: 0, total: 0, out }
    }

    fn compress(&mut self, last: bool) {
        let mut m = [0u64; 16];
        for (i, w) in m.iter_mut().enumerate() {
            *w = u64::from_le_bytes(self.buf[i * 8..i * 8 + 8].try_into().unwrap());
        }
        let mut v = [0u64; 16];
        v[..8].copy_from_slice(&self.h);
        v[8..].copy_from_slice(&IV);
        v[12] ^= self.total as u64;
        v[13] ^= (self.total >> 64) as u64;
        if last {
            v[14] = !v[14];
        }
        fn g(v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize, x: u64, y: u64) {
            v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
            v[d] = (v[d] ^ v[a]).rotate_right(32);
            v[c] = v[c].wrapping_add(v[d]);
            v[b] = (v[b] ^ v[c]).rotate_right(24);
            v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
            v[d] = (v[d] ^ v[a]).rotate_right(16);
            v[c] = v[c].wrapping_add(v[d]);
            v[b] = (v[b] ^ v[c]).rotate_right(63);
        }
        for s in &SIGMA {
            g(&mut v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
            g(&mut v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
            g(&mut v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
            g(&mut v, 3, 7, 11, 15, m[s[6]], m[s[7]]);
            g(&mut v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
            g(&mut v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
            g(&mut v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
            g(&mut v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
        }
        for i in 0..8 {
            self.h[i] ^= v[i] ^ v[i + 8];
        }
    }

    pub(super) fn update(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            if self.fill == 128 {
                // a full buffer is compressed only once more input arrives: the last block must carry the final flag
                self.total += 128;
                self.compress(false);
                self.fill = 0;
            }
            let take = (128 - self.fill).min(data.len());
            self.buf[self.fill..self.fill + take].copy_from_slice(&data[..take]);
            self.fill += take;
            data = &data[take..];
        }
    }

    pub(super) fn hexdigest(mut self) -> String {
        self.total += self.fill as u128;
        self.buf[self.fill..].fill(0);
        self.compress(true);
        let bytes: Vec<u8> = self.h.iter().flat_map(|w| w.to_le_bytes()).take(self.out).collect();
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// `Topology.digest` of the Python engine: blake2b(16) over vertex_point (int32), `|`, prim_offsets (int32), and when some
/// primitive is open `|open|` plus one byte per primitive.
pub(super) fn topology_digest(t: &Topology) -> String {
    let mut h = Blake2b::new(16);
    let vp: Vec<u8> = t.vertex_point.iter().flat_map(|x| x.to_le_bytes()).collect();
    h.update(&vp);
    h.update(b"|");
    let po: Vec<u8> = t.prim_offsets.iter().flat_map(|x| x.to_le_bytes()).collect();
    h.update(&po);
    if t.has_open() {
        h.update(b"|open|");
        let c: Vec<u8> = t.closed_mask().iter().map(|&b| b as u8).collect();
        h.update(&c);
    }
    h.hexdigest()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake2b_matches_hashlib() {
        // hashlib.blake2b(b"", digest_size=16).hexdigest() / blake2b(b"abc", digest_size=64)
        assert_eq!(Blake2b::new(16).hexdigest(), "cae66941d9efbd404e4d88758ea67670");
        let mut h = Blake2b::new(64);
        h.update(b"abc");
        assert_eq!(h.hexdigest(), "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923");
        // exactly one and a bit blocks
        let data: Vec<u8> = (0..200u32).map(|i| i as u8).collect();
        let mut h = Blake2b::new(16);
        h.update(&data[..128]);
        h.update(&data[128..]);
        let mut whole = Blake2b::new(16);
        whole.update(&data);
        assert_eq!(h.hexdigest(), "61479efa6267fea757b3f881e2979bbc");
        assert_eq!(whole.hexdigest(), "61479efa6267fea757b3f881e2979bbc");
    }
}
