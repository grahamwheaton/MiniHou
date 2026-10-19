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

/// 3D Simplex noise in roughly [-1, 1].
pub fn simplex3(p: [f32; 3], seed: i64) -> f32 {
    let mix = seed_mix(seed);
    const F3: f32 = 1.0 / 3.0;
    const G3: f32 = 1.0 / 6.0;
    let s = (p[0] + p[1] + p[2]) * F3;
    let i = (p[0] + s).floor();
    let j = (p[1] + s).floor();
    let k = (p[2] + s).floor();
    let t = (i + j + k) * G3;
    let x0 = p[0] - (i - t);
    let y0 = p[1] - (j - t);
    let z0 = p[2] - (k - t);

    let (i1, j1, k1, i2, j2, k2) = if x0 >= y0 {
        if y0 >= z0 { (1, 0, 0, 1, 1, 0) }
        else if x0 >= z0 { (1, 0, 0, 1, 0, 1) }
        else { (0, 0, 1, 1, 0, 1) }
    } else if y0 < z0 {
        (0, 0, 1, 0, 1, 1)
    } else if x0 < z0 {
        (0, 1, 0, 0, 1, 1)
    } else {
        (0, 1, 0, 1, 1, 0)
    };

    let x1 = x0 - i1 as f32 + G3;
    let y1 = y0 - j1 as f32 + G3;
    let z1 = z0 - k1 as f32 + G3;
    let x2 = x0 - i2 as f32 + 2.0 * G3;
    let y2 = y0 - j2 as f32 + 2.0 * G3;
    let z2 = z0 - k2 as f32 + 2.0 * G3;
    let x3 = x0 - 1.0 + 3.0 * G3;
    let y3 = y0 - 1.0 + 3.0 * G3;
    let z3 = z0 - 1.0 + 3.0 * G3;

    let ii = i as i32;
    let jj = j as i32;
    let kk = k as i32;

    let mut n = 0.0f32;
    let mut t0 = 0.6 - x0 * x0 - y0 * y0 - z0 * z0;
    if t0 > 0.0 {
        t0 *= t0;
        n += t0 * t0 * grad(hash3(ii, jj, kk, mix), x0, y0, z0);
    }
    let mut t1 = 0.6 - x1 * x1 - y1 * y1 - z1 * z1;
    if t1 > 0.0 {
        t1 *= t1;
        n += t1 * t1 * grad(hash3(ii.wrapping_add(i1), jj.wrapping_add(j1), kk.wrapping_add(k1), mix), x1, y1, z1);
    }
    let mut t2 = 0.6 - x2 * x2 - y2 * y2 - z2 * z2;
    if t2 > 0.0 {
        t2 *= t2;
        n += t2 * t2 * grad(hash3(ii.wrapping_add(i2), jj.wrapping_add(j2), kk.wrapping_add(k2), mix), x2, y2, z2);
    }
    let mut t3 = 0.6 - x3 * x3 - y3 * y3 - z3 * z3;
    if t3 > 0.0 {
        t3 *= t3;
        n += t3 * t3 * grad(hash3(ii.wrapping_add(1), jj.wrapping_add(1), kk.wrapping_add(1), mix), x3, y3, z3);
    }
    (n * 32.0).clamp(-1.0, 1.0)
}

/// 3D Worley (cellular F1) noise in roughly [-1, 1].
pub fn worley3(p: [f32; 3], seed: i64) -> f32 {
    let mix = seed_mix(seed);
    let xi = p[0].floor() as i32;
    let yi = p[1].floor() as i32;
    let zi = p[2].floor() as i32;
    let mut min_dist_sq = f32::INFINITY;
    for dx in -1..=1 {
        for dy in -1..=1 {
            for dz in -1..=1 {
                let cx = xi.wrapping_add(dx);
                let cy = yi.wrapping_add(dy);
                let cz = zi.wrapping_add(dz);
                let hx = hash3(cx, cy, cz, mix);
                let hy = hash3(cx, cy, cz, mix ^ 0x5555_5555);
                let hz = hash3(cx, cy, cz, mix ^ 0xAAAA_AAAA);
                let fx = (cx as f32) + (hx as f32 / u32::MAX as f32);
                let fy = (cy as f32) + (hy as f32 / u32::MAX as f32);
                let fz = (cz as f32) + (hz as f32 / u32::MAX as f32);
                let dist_sq = (p[0] - fx) * (p[0] - fx) + (p[1] - fy) * (p[1] - fy) + (p[2] - fz) * (p[2] - fz);
                if dist_sq < min_dist_sq {
                    min_dist_sq = dist_sq;
                }
            }
        }
    }
    let d = min_dist_sq.sqrt().min(1.0);
    d * 2.0 - 1.0
}

/// Periodic Perlin noise wrapped with integer period.
pub fn periodic_perlin3(p: [f32; 3], seed: i64, period: i32) -> f32 {
    let mix = seed_mix(seed);
    let period = if period > 0 { period } else { 16 };
    let fl = [p[0].floor(), p[1].floor(), p[2].floor()];
    let f = [p[0] - fl[0], p[1] - fl[1], p[2] - fl[2]];
    let i = [fl[0] as i32 % period, fl[1] as i32 % period, fl[2] as i32 % period];
    let i = [
        if i[0] < 0 { i[0] + period } else { i[0] },
        if i[1] < 0 { i[1] + period } else { i[1] },
        if i[2] < 0 { i[2] + period } else { i[2] },
    ];
    let u = [fade(f[0]), fade(f[1]), fade(f[2])];
    let corner = |dx: i32, dy: i32, dz: i32| {
        let ix = (i[0] + dx) % period;
        let iy = (i[1] + dy) % period;
        let iz = (i[2] + dz) % period;
        grad(hash3(ix, iy, iz, mix), f[0] - dx as f32, f[1] - dy as f32, f[2] - dz as f32)
    };
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u[0]);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u[0]);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u[0]);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u[0]);
    lerp(lerp(x00, x10, u[1]), lerp(x01, x11, u[1]), u[2])
}

/// Dispatcher for raw 3D noise types.
pub fn sample_noise(noise_type: &str, p: [f32; 3], seed: i64) -> f32 {
    match noise_type {
        "simplex" => simplex3(p, seed),
        "worley" => worley3(p, seed),
        "periodic" => periodic_perlin3(p, seed, 8),
        _ => perlin3(p, seed),
    }
}

/// Fractal summation supporting standard fBm, rigid terrain, and hybrid terrain.
pub fn sample_fractal(noise_type: &str, fractal_type: &str, p: [f32; 3], seed: i64, octaves: u32, lacunarity: f64, gain: f64) -> f32 {
    match fractal_type {
        "terrain" => {
            let mut total = 0.0f32;
            let (mut amp, mut freq, mut norm) = (1.0f64, 1.0f64, 0.0f64);
            for o in 0..octaves {
                let fq = freq as f32;
                let val = sample_noise(noise_type, [p[0] * fq, p[1] * fq, p[2] * fq], seed + o as i64 * 7919);
                let rigid = 1.0 - val.abs();
                total += (amp as f32) * (rigid * rigid);
                norm += amp;
                amp *= gain;
                freq *= lacunarity;
            }
            if norm > 0.0 { (total / norm as f32) * 2.0 - 1.0 } else { total }
        }
        "hybrid_terrain" => {
            let mut total = 0.0f32;
            let (mut amp, mut freq, mut norm) = (1.0f64, 1.0f64, 0.0f64);
            let mut weight = 1.0f32;
            for o in 0..octaves {
                let fq = freq as f32;
                let val = sample_noise(noise_type, [p[0] * fq, p[1] * fq, p[2] * fq], seed + o as i64 * 7919);
                total += (amp as f32) * val * weight;
                weight = (weight * (val + 1.0) * 0.5).clamp(0.0, 1.0);
                norm += amp;
                amp *= gain;
                freq *= lacunarity;
            }
            if norm > 0.0 { total / norm as f32 } else { total }
        }
        _ => {
            if noise_type == "perlin" {
                fbm3_one(p, seed, octaves, lacunarity, gain)
            } else {
                let mut total = 0.0f32;
                let (mut amp, mut freq, mut norm) = (1.0f64, 1.0f64, 0.0f64);
                for o in 0..octaves {
                    let fq = freq as f32;
                    total += (amp as f32) * sample_noise(noise_type, [p[0] * fq, p[1] * fq, p[2] * fq], seed + o as i64 * 7919);
                    norm += amp;
                    amp *= gain;
                    freq *= lacunarity;
                }
                if norm > 0.0 { total / norm as f32 } else { total }
            }
        }
    }
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
