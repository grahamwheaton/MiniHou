//! Heightfield erosion kernels: thermal and hydraulic.
//!
//! Matches the arithmetic and operations of `minihou.nodes.heightfield`.

use rayon::prelude::*;

pub const NEIGHBOURS: [(isize, isize); 8] = [
    (-1, -1), (-1, 0), (-1, 1),
    (0, -1),           (0, 1),
    (1, -1),  (1, 0),  (1, 1),
];

const BRUSH_OFFSETS: [(isize, isize); 9] = [
    (-1, -1), (-1, 0), (-1, 1),
    (0, -1),  (0, 0),  (0, 1),
    (1, -1),  (1, 0),  (1, 1),
];

fn brush_factors() -> [(isize, isize, f64); 9] {
    let mut factors = [(0, 0, 0.0); 9];
    let mut bsum = 0.0;
    for (i, &(dy, dx)) in BRUSH_OFFSETS.iter().enumerate() {
        let hyp = ((dy * dy + dx * dx) as f64).sqrt();
        let wgt = (1.5 - hyp).max(0.0);
        factors[i] = (dy, dx, wgt);
        bsum += wgt;
    }
    for item in &mut factors {
        item.2 /= bsum;
    }
    factors
}

/// Material steeper than the talus angle slides to lower neighbours.
pub fn thermal(
    h: &[f64],
    rows: usize,
    cols: usize,
    tan: f64,
    dist: &[f64],
    rate: f64,
    iterations: usize,
) -> Vec<f64> {
    assert_eq!(h.len(), rows * cols);
    assert_eq!(dist.len(), 8);
    let mut h_curr = h.to_vec();
    if iterations == 0 || rows == 0 || cols == 0 {
        return h_curr;
    }

    let mut move_buf = vec![0.0; rows * cols];
    let mut share_buf = vec![[0.0; 8]; rows * cols];

    for _ in 0..iterations {
        // Step 1: compute move and share for all cells
        move_buf
            .par_chunks_exact_mut(cols)
            .zip(share_buf.par_chunks_exact_mut(cols))
            .enumerate()
            .for_each(|(y, (m_row, s_row))| {
                for x in 0..cols {
                    let idx = y * cols + x;
                    let hc = h_curr[idx];
                    let mut tot = 0.0;
                    let mut max_ex = 0.0;
                    let mut sh = [0.0; 8];
                    for (k, &(dy, dx)) in NEIGHBOURS.iter().enumerate() {
                        let ny = y as isize + dy;
                        let nx = x as isize + dx;
                        let ex = if ny >= 0 && ny < rows as isize && nx >= 0 && nx < cols as isize {
                            let nb = h_curr[ny as usize * cols + nx as usize];
                            let drop = hc - nb;
                            (drop - tan * dist[k]).max(0.0)
                        } else {
                            0.0
                        };
                        sh[k] = ex;
                        tot += ex;
                        if ex > max_ex {
                            max_ex = ex;
                        }
                    }
                    let mv = (rate * 0.5) * max_ex;
                    m_row[x] = mv;
                    if tot > 0.0 {
                        for item in &mut sh {
                            *item = (*item / tot) * mv;
                        }
                    } else {
                        sh = [0.0; 8];
                    }
                    s_row[x] = sh;
                }
            });

        // Step 2: update h_curr
        h_curr
            .par_chunks_exact_mut(cols)
            .enumerate()
            .for_each(|(y, h_row)| {
                for (x, h_val) in h_row.iter_mut().enumerate() {
                    let idx = y * cols + x;
                    let mut val = *h_val - move_buf[idx];
                    for (k, &(dy, dx)) in NEIGHBOURS.iter().enumerate() {
                        let sy = y as isize - dy;
                        let sx = x as isize - dx;
                        if sy >= 0 && sy < rows as isize && sx >= 0 && sx < cols as isize {
                            val += share_buf[sy as usize * cols + sx as usize][k];
                        }
                    }
                    *h_val = val;
                }
            });
    }

    h_curr
}

#[inline(always)]
fn bilinear_sample(
    h: &[f64],
    rows: usize,
    cols: usize,
    x: f64,
    y: f64,
) -> (f64, f64, f64, usize, usize, f64, f64) {
    let x0 = (x.floor() as i64).clamp(0, cols as i64 - 2) as usize;
    let y0 = (y.floor() as i64).clamp(0, rows as i64 - 2) as usize;
    let fx = x - x0 as f64;
    let fy = y - y0 as f64;
    let a = h[y0 * cols + x0];
    let b = h[y0 * cols + (x0 + 1)];
    let c = h[(y0 + 1) * cols + x0];
    let d = h[(y0 + 1) * cols + (x0 + 1)];
    let height = a * (1.0 - fx) * (1.0 - fy) + b * fx * (1.0 - fy) + c * (1.0 - fx) * fy + d * fx * fy;
    let gx = (b - a) * (1.0 - fy) + (d - c) * fy;
    let gy = (c - a) * (1.0 - fx) + (d - b) * fx;
    (height, gx, gy, x0, y0, fx, fy)
}

#[inline(always)]
fn bilinear_height_only(
    h: &[f64],
    rows: usize,
    cols: usize,
    x: f64,
    y: f64,
) -> f64 {
    let x0 = (x.floor() as i64).clamp(0, cols as i64 - 2) as usize;
    let y0 = (y.floor() as i64).clamp(0, rows as i64 - 2) as usize;
    let fx = x - x0 as f64;
    let fy = y - y0 as f64;
    let a = h[y0 * cols + x0];
    let b = h[y0 * cols + (x0 + 1)];
    let c = h[(y0 + 1) * cols + x0];
    let d = h[(y0 + 1) * cols + (x0 + 1)];
    a * (1.0 - fx) * (1.0 - fy) + b * fx * (1.0 - fy) + c * (1.0 - fx) * fy + d * fx * fy
}

/// State for hydraulic droplet erosion across iterations.
pub struct HydraulicState {
    pub rows: usize,
    pub cols: usize,
    pub h: Vec<f64>,
    pub flow: Vec<f64>,
    pub eroded: Vec<f64>,
    pub deposited: Vec<f64>,
    want: Vec<f64>,
    allow: Vec<f64>,
    touched: Vec<usize>,
}

impl HydraulicState {
    pub fn new(h: Vec<f64>, rows: usize, cols: usize) -> Self {
        let n = rows * cols;
        assert_eq!(h.len(), n);
        HydraulicState {
            rows,
            cols,
            h,
            flow: vec![0.0; n],
            eroded: vec![0.0; n],
            deposited: vec![0.0; n],
            want: vec![0.0; n],
            allow: vec![1.0; n],
            touched: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        x_init: &[f64],
        y_init: &[f64],
        cell: (f64, f64),
        rain: f64,
        capacity: f64,
        erode_rate: f64,
        deposit_rate: f64,
        evaporation: f64,
        lifetime: usize,
        inertia: f64,
        gravity: f64,
        min_slope: f64,
    ) {
        let droplets = x_init.len();
        assert_eq!(y_init.len(), droplets);
        if droplets == 0 || self.rows < 2 || self.cols < 2 {
            return;
        }

        let w = self.cols;
        let h = self.rows;
        let w_f = w as f64;
        let h_f = h as f64;
        let brush = brush_factors();
        let water0 = rain.max(1e-6) * 100.0;

        let mut x = x_init.to_vec();
        let mut y = y_init.to_vec();
        let mut dx = vec![0.0; droplets];
        let mut dy = vec![0.0; droplets];
        let mut vel = vec![1.0; droplets];
        let mut water = vec![water0; droplets];
        let mut sed = vec![0.0; droplets];
        let mut alive = vec![true; droplets];
        let mut active: Vec<usize> = (0..droplets).collect();

        // Scratch buffers per active droplet
        let mut hx_buf = vec![0.0; droplets];
        let mut gx_buf = vec![0.0; droplets];
        let mut gy_buf = vec![0.0; droplets];
        let mut x0_buf = vec![0; droplets];
        let mut y0_buf = vec![0; droplets];
        let mut fx_buf = vec![0.0; droplets];
        let mut fy_buf = vec![0.0; droplets];
        let mut ndx_buf = vec![0.0; droplets];
        let mut ndy_buf = vec![0.0; droplets];
        let mut nx_buf = vec![0.0; droplets];
        let mut ny_buf = vec![0.0; droplets];
        let mut inside_buf = vec![false; droplets];
        let mut dh_buf = vec![0.0; droplets];
        let mut dep_buf = vec![0.0; droplets];
        let mut ero_buf = vec![0.0; droplets];
        let mut cy_buf = vec![0; droplets];
        let mut cx_buf = vec![0; droplets];

        for _step in 0..lifetime {
            if active.is_empty() {
                break;
            }
            let n_act = active.len();

            // 1. Bilinear sampling and direction calculation
            for (i, &k) in active.iter().enumerate() {
                let (hx, gx, gy, x0, y0, fx, fy) = bilinear_sample(&self.h, h, w, x[k], y[k]);
                hx_buf[i] = hx;
                gx_buf[i] = gx;
                gy_buf[i] = gy;
                x0_buf[i] = x0;
                y0_buf[i] = y0;
                fx_buf[i] = fx;
                fy_buf[i] = fy;

                let mut ndx_k = dx[k] * inertia - gx * (1.0 - inertia);
                let mut ndy_k = dy[k] * inertia - gy * (1.0 - inertia);
                let ln = (ndx_k * ndx_k + ndy_k * ndy_k).sqrt();
                let moving = ln > 1e-12;
                if moving {
                    ndx_k /= ln;
                    ndy_k /= ln;
                } else {
                    ndx_k = 0.0;
                    ndy_k = 0.0;
                }
                let nx_k = x[k] + ndx_k;
                let ny_k = y[k] + ndy_k;
                let inside_k = moving && nx_k >= 0.0 && nx_k <= w_f - 1.001 && ny_k >= 0.0 && ny_k <= h_f - 1.001;

                ndx_buf[i] = ndx_k;
                ndy_buf[i] = ndy_k;
                nx_buf[i] = nx_k;
                ny_buf[i] = ny_k;
                inside_buf[i] = inside_k;

                self.flow[y0 * w + x0] += water[k];
            }

            // 2. Sample new height, compute dh, cap, amount_dep, amount_ero
            for (i, &k) in active.iter().enumerate() {
                let inside_k = inside_buf[i];
                let cnx = nx_buf[i].clamp(0.0, w_f - 1.001);
                let cny = ny_buf[i].clamp(0.0, h_f - 1.001);
                let nh = bilinear_height_only(&self.h, h, w, cnx, cny);
                let dh = if inside_k { nh - hx_buf[i] } else { 0.0 };
                dh_buf[i] = dh;

                let cap = (-dh).max(min_slope * cell.1) * vel[k] * water[k] * capacity;
                let depositing = sed[k] > cap || dh > 0.0 || !inside_k;
                let mut amount_dep = if !inside_k {
                    sed[k]
                } else if dh > 0.0 {
                    dh.min(sed[k])
                } else {
                    (sed[k] - cap) * deposit_rate
                };
                if depositing {
                    if amount_dep < 0.0 {
                        amount_dep = 0.0;
                    }
                } else {
                    amount_dep = 0.0;
                }

                let mut amount_ero = if depositing {
                    0.0
                } else {
                    ((cap - sed[k]) * erode_rate).min(-dh)
                };
                if amount_ero < 0.0 {
                    amount_ero = 0.0;
                }

                dep_buf[i] = amount_dep;
                ero_buf[i] = amount_ero;
            }

            // 3. Bilinear deposit for 4 corners in order
            for &(ox, oy, corner_idx) in &[(0, 0, 0), (1, 0, 1), (0, 1, 2), (1, 1, 3)] {
                for i in 0..n_act {
                    let wgt = match corner_idx {
                        0 => (1.0 - fx_buf[i]) * (1.0 - fy_buf[i]),
                        1 => fx_buf[i] * (1.0 - fy_buf[i]),
                        2 => (1.0 - fx_buf[i]) * fy_buf[i],
                        _ => fx_buf[i] * fy_buf[i],
                    };
                    let val = dep_buf[i] * wgt;
                    let idx = (y0_buf[i] + oy) * w + (x0_buf[i] + ox);
                    self.h[idx] += val;
                    self.deposited[idx] += val;
                }
            }

            // 4. Brush erosion
            for (i, &k) in active.iter().enumerate() {
                cy_buf[i] = (y[k].round_ties_even() as i64).clamp(0, h as i64 - 1) as usize;
                cx_buf[i] = (x[k].round_ties_even() as i64).clamp(0, w as i64 - 1) as usize;
            }

            for &(by, bx, factor) in &brush {
                for i in 0..n_act {
                    let yy = (cy_buf[i] as isize + by).clamp(0, h as isize - 1) as usize;
                    let xx = (cx_buf[i] as isize + bx).clamp(0, w as isize - 1) as usize;
                    let take = ero_buf[i] * factor;
                    let idx = yy * w + xx;
                    self.want[idx] += take;
                    self.touched.push(idx);
                }
            }

            self.touched.sort_unstable();
            self.touched.dedup();

            // 4a. Compute room and allow for each touched cell
            for &idx in &self.touched {
                let yy = idx / w;
                let xx = idx % w;
                let mut min_nb = f64::INFINITY;
                for &(dy_nb, dx_nb) in &NEIGHBOURS {
                    let ny = yy as isize + dy_nb;
                    let nx = xx as isize + dx_nb;
                    if ny >= 0 && ny < h as isize && nx >= 0 && nx < w as isize {
                        let nb = self.h[ny as usize * w + nx as usize];
                        if nb < min_nb {
                            min_nb = nb;
                        }
                    }
                }
                let room = 0.5 * (self.h[idx] - min_nb).max(0.0);
                let w_val = self.want[idx];
                let a_val = if w_val > room {
                    room / w_val
                } else {
                    1.0
                };
                self.allow[idx] = a_val;
            }

            // 4b. Update H and eroded
            for &idx in &self.touched {
                let prod = self.want[idx] * self.allow[idx];
                self.h[idx] -= prod;
                self.eroded[idx] += prod;
            }

            // 4c. Compute new amount_ero for droplets and update sed
            for (i, &k) in active.iter().enumerate() {
                let mut ero_new = 0.0;
                for &(by, bx, factor) in &brush {
                    let yy = (cy_buf[i] as isize + by).clamp(0, h as isize - 1) as usize;
                    let xx = (cx_buf[i] as isize + bx).clamp(0, w as isize - 1) as usize;
                    let take = ero_buf[i] * factor;
                    ero_new += take * self.allow[yy * w + xx];
                }
                sed[k] += ero_new - dep_buf[i];
            }

            // 4d. Reset want and allow for touched cells
            for &idx in &self.touched {
                self.want[idx] = 0.0;
                self.allow[idx] = 1.0;
            }
            self.touched.clear();

            // 5. Update physics
            for (i, &k) in active.iter().enumerate() {
                vel[k] = (vel[k] * vel[k] - dh_buf[i] * gravity).max(0.0).sqrt();
                water[k] *= 1.0 - evaporation;
                dx[k] = ndx_buf[i];
                dy[k] = ndy_buf[i];
                x[k] = nx_buf[i];
                y[k] = ny_buf[i];
                alive[k] = inside_buf[i] && (water[k] > water0 * 0.01);
            }

            active.retain(|&k| alive[k]);
        }
    }

    pub fn finish(self) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
        (self.h, self.flow, self.eroded, self.deposited)
    }
}
