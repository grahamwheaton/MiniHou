//! Gradient (Perlin) noise and its fractal sum, bit for bit the float32 arithmetic of `minihou.nodes._util.perlin3` /
//! `fbm3_numpy` (same lattice hash, same operation order; Rust never fuses multiply-adds on its own).

use rayon::prelude::*;

#[inline]
fn hash3(ix: i32, iy: i32, iz: i32, seed_mix: u32) -> u32 {
    let mut h = (ix as u32).wrapping_mul(0x8DA6_B343);
    h ^= (iy as u32).wrapping_mul(0xD816_3841);
    h ^= (iz as u32).wrapping_mul(0xCB1A_B31F);
    h ^= seed_mix;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    h
}

/// `(seed * 0x9E3779B1) & 0xFFFFFFFF` with Python's unbounded integers.
pub fn seed_mix(seed: i64) -> u32 {
    ((seed as i128).wrapping_mul(0x9E37_79B1) & 0xFFFF_FFFF) as u32
}

#[inline]
fn grad(h: u32, x: f32, y: f32, z: f32) -> f32 {
    let h = h & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 { y } else if h == 12 || h == 14 { x } else { z };
    (if h & 1 != 0 { -u } else { u }) + (if h & 2 != 0 { -v } else { v })
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + t * (b - a)
}

#[inline]
fn fade(f: f32) -> f32 {
    f * f * f * (f * (f * 6.0 - 15.0) + 10.0)
}

/// Noise at one position, roughly in [-1, 1].
pub fn perlin3(p: [f32; 3], seed: i64) -> f32 {
    let mix = seed_mix(seed);
    let fl = [p[0].floor(), p[1].floor(), p[2].floor()];
    let f = [p[0] - fl[0], p[1] - fl[1], p[2] - fl[2]];
    let i = [fl[0] as i32, fl[1] as i32, fl[2] as i32];
    let u = [fade(f[0]), fade(f[1]), fade(f[2])];
    let corner = |dx: i32, dy: i32, dz: i32| {
        grad(hash3(i[0].wrapping_add(dx), i[1].wrapping_add(dy), i[2].wrapping_add(dz), mix),
             f[0] - dx as f32, f[1] - dy as f32, f[2] - dz as f32)
    };
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u[0]);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u[0]);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u[0]);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u[0]);
    lerp(lerp(x00, x10, u[1]), lerp(x01, x11, u[1]), u[2])
}

/// Fractal sum of `octaves` noise layers at one position, normalised to stay near [-1, 1].
pub fn fbm3_one(p: [f32; 3], seed: i64, octaves: u32, lacunarity: f64, gain: f64) -> f32 {
    let mut total = 0.0f32;
    let (mut amp, mut freq, mut norm) = (1.0f64, 1.0f64, 0.0f64);
    for o in 0..octaves {
        let fq = freq as f32;
        total += (amp as f32) * perlin3([p[0] * fq, p[1] * fq, p[2] * fq], seed + o as i64 * 7919);
        norm += amp;
        amp *= gain;
        freq *= lacunarity;
    }
    if norm > 0.0 { total / norm as f32 } else { total }
}

/// [`fbm3_one`] for every row of `pos` (x, y, z per point), in parallel.
pub fn fbm3(pos: &[f32], seed: i64, octaves: u32, lacunarity: f64, gain: f64) -> Vec<f32> {
    pos.par_chunks_exact(3)
        .map(|c| fbm3_one([c[0], c[1], c[2]], seed, octaves, lacunarity, gain))
        .collect()
}

/// The `noise` node's vector work in one parallel pass, bit for bit its numpy arithmetic: sample at `P * frequency + offset`,
/// one fractal value times `amplitude` (scalar) or three decorrelated ones (`vector`, seeds `seed + 1013 * c`), and add the
/// result to `P` (`add`) or return it on its own.
#[allow(clippy::too_many_arguments)]
pub fn noise_vec(p: &[f32], frequency: f32, offset: [f32; 3], amplitude: [f32; 3], seed: i64, octaves: u32, lacunarity: f64,
                 gain: f64, vector: bool, add: bool) -> Vec<f32> {
    let mut out = vec![0.0f32; p.len()];
    out.par_chunks_exact_mut(3).zip(p.par_chunks_exact(3)).for_each(|(o, q)| {
        let pos = [q[0] * frequency + offset[0], q[1] * frequency + offset[1], q[2] * frequency + offset[2]];
        let v = if vector {
            [0, 1, 2].map(|c| fbm3_one(pos, seed + 1013 * c as i64, octaves, lacunarity, gain) * amplitude[c])
        } else {
            let n = fbm3_one(pos, seed, octaves, lacunarity, gain);
            [n * amplitude[0], n * amplitude[1], n * amplitude[2]]
        };
        for k in 0..3 {
            o[k] = if add { q[k] + v[k] } else { v[k] };
        }
    });
    out
}
