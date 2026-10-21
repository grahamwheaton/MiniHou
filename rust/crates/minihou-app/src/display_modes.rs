//! Stable display names, legacy preference mapping and ownership partitioning.
use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    WireBoundingBox, Wireframe, WireframeGhost,
    HiddenLineInvisible, HiddenLineGhost, MatCapShaded,
    FlatShaded, #[default] SmoothShaded,
}
impl Mode {
    pub const ALL: [Self; 8] = [Self::WireBoundingBox, Self::Wireframe,
        Self::WireframeGhost, Self::HiddenLineInvisible, Self::HiddenLineGhost, Self::MatCapShaded,
        Self::FlatShaded, Self::SmoothShaded];
    // Source compatibility for the existing Z pie and callers.
    #[allow(non_upper_case_globals)] pub const Solid: Self = Self::FlatShaded;
    #[allow(non_upper_case_globals)] pub const Shaded: Self = Self::SmoothShaded;
    #[allow(non_upper_case_globals)] pub const Wire: Self = Self::Wireframe;
    pub fn label(self) -> &'static str { match self {
        Self::WireBoundingBox => "Wireframe Bounding Box",
        Self::Wireframe => "Wireframe", Self::WireframeGhost => "Wireframe Ghost",
        Self::HiddenLineInvisible => "Hidden Line Invisible", Self::HiddenLineGhost => "Hidden Line Ghost",
        Self::MatCapShaded => "MatCap Shaded",
        Self::FlatShaded => "Flat Shaded",
        Self::SmoothShaded => "Smooth Shaded",
    } }
    pub fn parse(s: &str) -> Self {
        match s { "Solid" => Self::Solid, "Shaded" => Self::Shaded, "Wire" => Self::Wire,
            _ => Self::ALL.into_iter().find(|m| m.label() == s).unwrap_or_default() }
    }
    pub fn cycle(self, forward: bool) -> Self {
        let i = Self::ALL.iter().position(|m| *m == self).unwrap();
        Self::ALL[(i + if forward { 1 } else { Self::ALL.len() - 1 }) % Self::ALL.len()]
    }
    pub fn bounds(self) -> bool { self == Self::WireBoundingBox }
    pub fn flat(self) -> bool { self == Self::FlatShaded }
    pub fn matcap(self) -> bool { self == Self::MatCapShaded }
    pub fn shaded(self) -> bool { matches!(self, Self::MatCapShaded | Self::FlatShaded | Self::SmoothShaded) }
    pub fn wire(self) -> bool { !self.shaded() }
    pub fn hidden(self) -> bool { matches!(self, Self::HiddenLineInvisible | Self::HiddenLineGhost) }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ObjectVisibility { #[default] ShowAll, GhostOthers, HideOthers }
impl ObjectVisibility {
    pub const ALL: [Self; 3] = [Self::ShowAll, Self::GhostOthers, Self::HideOthers];
    pub fn label(self) -> &'static str { match self {
        Self::ShowAll => "Show All Objects", Self::GhostOthers => "Ghost Other Objects", Self::HideOthers => "Hide Other Objects",
    } }
    pub fn parse(s: &str) -> Self { Self::ALL.into_iter().find(|m| m.label() == s).unwrap_or_default() }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MatCap { #[default] Clay, Normal, SketchUpClean }
impl MatCap {
    pub const ALL: [Self; 3] = [Self::Clay, Self::Normal, Self::SketchUpClean];
    pub fn label(self) -> &'static str { match self { Self::Clay => "Clay", Self::Normal => "Normal", Self::SketchUpClean => "SketchUp clean" } }
    pub fn parse(s: &str) -> Self { Self::ALL.into_iter().find(|v| v.label()==s).unwrap_or_default() }
    pub fn texture_id(self) -> u8 { match self { Self::Clay => 1, Self::Normal => 2, Self::SketchUpClean => 3 } }
    pub fn pixels(self, size:u32) -> Vec<u8> { match self { Self::Clay => clay_matcap(size), Self::Normal => normal_matcap(size), Self::SketchUpClean => sketchup_clean_matcap(size) } }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ObjectColor { #[default] Single, Random }
impl ObjectColor {
    pub const ALL: [Self; 2] = [Self::Single, Self::Random];
    pub fn label(self) -> &'static str { match self { Self::Single => "Single", Self::Random => "Random" } }
    pub fn parse(s: &str) -> Self { Self::ALL.into_iter().find(|v| v.label()==s).unwrap_or_default() }
}

/// FNV-1a uses fixed constants, so object colours do not depend on process hash seeds.
pub fn random_object_color(identity: &str) -> [f32; 3] {
    let hash = identity.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    let hue = (hash as u32 as f64 / 4294967296.0) as f32;
    let x = hue * 6.; let f = x.fract();
    let (r,g,b) = match x as u32 { 0 => (1.,f,0.), 1 => (1.-f,1.,0.), 2 => (0.,1.,f),
        3 => (0.,1.-f,1.), 4 => (f,0.,1.), _ => (1.,0.,1.-f) };
    // Linear RGB with fixed saturation/value: bright enough under the studio rig.
    [r,g,b].map(|c| 0.48 + 0.32*c)
}

pub fn normal_color(n: [f32; 3]) -> [f32; 3] { n.map(|c| c*0.5+0.5) }

pub fn normal_matcap(size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size*size*4) as usize);
    for y in 0..size { for x in 0..size {
        let nx = 2.*(x as f32+0.5)/size as f32-1.;
        let ny = 1.-2.*(y as f32+0.5)/size as f32;
        let nz = (1.-nx*nx-ny*ny).max(0.).sqrt();
        pixels.extend(normal_color([nx,ny,nz]).map(|v| (v.clamp(0.,1.)*255.).round() as u8));
        pixels.push(if nx*nx+ny*ny<=1. {255} else {0});
    } }
    pixels
}

/// Neutral paper-white capture with a broad, restrained change in value across face normals.
pub fn sketchup_clean_matcap(size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size*size*4) as usize);
    for y in 0..size { for x in 0..size {
        let nx = 2.*(x as f32+0.5)/size as f32-1.;
        let ny = 1.-2.*(y as f32+0.5)/size as f32;
        let nz = (1.-nx*nx-ny*ny).max(0.).sqrt();
        let key = (-0.35*nx + 0.55*ny + 0.76*nz).max(0.);
        let value = (0.76 + 0.19*key + 0.04*ny).clamp(0.,1.);
        let grey = (value*255.).round() as u8;
        pixels.extend_from_slice(&[grey,grey,grey,255]);
    } }
    pixels
}

/// Partition whole primitives; retain original vertex IDs, including in packed LOD cuts.
pub fn split_indices(indices: &[u32], width: usize, selected: &[Range<u32>]) -> (Vec<u32>, Vec<u32>) {
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for primitive in indices.chunks_exact(width) {
        if selected.iter().any(|r| primitive.iter().all(|i| r.contains(i))) { a.extend_from_slice(primitive); }
        else { b.extend_from_slice(primitive); }
    }
    (a, b)
}

/// Linear RGB sphere capture: broad softbox, warm clay, cool fill and a soft rim.
pub fn clay_matcap(size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size { for x in 0..size {
        let nx = 2. * (x as f32 + 0.5) / size as f32 - 1.;
        let ny = 1. - 2. * (y as f32 + 0.5) / size as f32;
        let nz = (1. - nx*nx - ny*ny).max(0.).sqrt();
        let key = (-0.4*nx + 0.6*ny + 0.69*nz).max(0.);
        let fill = (0.65*nx - 0.2*ny + 0.73*nz).max(0.);
        let softbox = (-0.28*nx + 0.48*ny + 0.83*nz).max(0.).powf(24.);
        let rim = (1. - nz).powf(3.) * (0.5 + 0.5*ny);
        for c in [0.66, 0.63, 0.59] {
            let v = c * (0.19 + 0.64*key + 0.18*fill) + 0.22*softbox + 0.1*rim;
            pixels.push((v.clamp(0.,1.)*255.).round() as u8);
        }
        pixels.push(255);
    } }
    pixels
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn modes_round_trip_and_cycle() {
        for m in Mode::ALL { assert_eq!(Mode::parse(m.label()), m); assert_eq!(m.cycle(true).cycle(false), m); }
        assert_eq!(Mode::ALL[7].cycle(true), Mode::ALL[0]);
        for (old, new) in [("Solid",Mode::FlatShaded),("Shaded",Mode::SmoothShaded),("Wire",Mode::Wireframe)] { assert_eq!(Mode::parse(old),new); }
        for removed in ["Shaded Bounding Box", "MatCap Wire Shaded", "Flat Wire Shaded", "Smooth Wire Shaded", "ShadedWire", "Shaded + Wire"] {
            assert_eq!(Mode::parse(removed), Mode::SmoothShaded);
        }
        assert_eq!(Mode::parse("unknown"),Mode::SmoothShaded);
    }
    #[test] fn ownership_partition_survives_reordered_lod_indices() {
        let (a,b)=split_indices(&[7,8,9, 1,2,3, 10,11,12, 4,5,6],3,&[1..7]);
        assert_eq!(a,[1,2,3,4,5,6]); assert_eq!(b,[7,8,9,10,11,12]);
        assert_eq!(split_indices(&[1,2,8,9],2,&[1..7]),(vec![1,2],vec![8,9]));
        assert!(split_indices(&[1,2,3],3,&[]).0.is_empty());
    }
    #[test] fn matcap_has_softbox_and_opaque_pixels() {
        let p=clay_matcap(128); assert_eq!(p.len(),128*128*4);
        assert!(p.chunks_exact(4).all(|c| c[3]==255));
        assert!(p.chunks_exact(4).map(|c| c[0]).max().unwrap() > 200);
    }
    #[test] fn matcap_menu_includes_sketchup_clean_capture() {
        assert_eq!(MatCap::ALL.map(MatCap::label), ["Clay", "Normal", "SketchUp clean"]);
        for matcap in MatCap::ALL { assert_eq!(MatCap::parse(matcap.label()), matcap); }
        let pixels=MatCap::SketchUpClean.pixels(128);
        assert_eq!(pixels.len(),128*128*4);
        assert!(pixels.chunks_exact(4).all(|p| p[0]==p[1] && p[1]==p[2] && p[3]==255));
        assert_ne!(&pixels[(64*128+64)*4..(64*128+64)*4+3],&pixels[(8*128+64)*4..(8*128+64)*4+3]);
    }
    #[test] fn normal_capture_directions_and_object_hash() {
        assert_eq!(normal_color([0.,0.,1.]),[0.5,0.5,1.]);
        assert_eq!(normal_color([1.,0.,0.]),[1.,0.5,0.5]);
        assert_eq!(normal_color([0.,1.,0.]),[0.5,1.,0.5]);
        let p=normal_matcap(129); let c=&p[((64*129+64)*4)..][..4];
        assert_eq!(c,[128,128,255,255]);
        assert_eq!(random_object_color("box_A"),random_object_color("box_A"));
        assert_ne!(random_object_color("box_A"),random_object_color("box_B"));
        for name in ["", "box_A", "box_B", "unicode ✓"] {
            let c=random_object_color(name);
            assert!(c.iter().all(|v| (0.48..=0.8).contains(v)));
            assert!(c.iter().any(|v| *v>=0.79));
        }
    }
}
