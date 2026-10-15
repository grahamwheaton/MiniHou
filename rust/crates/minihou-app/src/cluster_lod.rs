//! CPU-only, group-atomic clustered LOD experiment for immutable cooked meshes.
//! Positions are shared by every level; edge collapses retain an existing endpoint.

use std::collections::{HashMap, HashSet};

pub const MAX_TRIANGLES: usize = 128;
pub type ClusterId = u32;
pub type NodeId = u32;
type Tri = [u32; 3];

#[derive(Clone, Copy, Debug, Default)]
pub struct Sphere { pub center: [f32; 3], pub radius: f32 }

#[derive(Clone, Debug)]
pub struct Cluster {
    pub triangles: Vec<Tri>,
    pub bounds: Sphere,
    pub error: f32,
    pub level: u32,
}

#[derive(Clone, Debug)]
pub struct DagNode {
    /// A group is selected in full: all of these clusters replace all children.
    pub clusters: Vec<ClusterId>,
    pub children: Vec<NodeId>,
    pub bounds: Sphere,
    pub error: f32,
    pub level: u32,
}

#[derive(Clone, Debug, Default)]
pub struct ClusterLod {
    pub positions: Vec<[f32; 3]>,
    pub clusters: Vec<Cluster>,
    pub nodes: Vec<DagNode>,
    /// A forest is necessary if a topology or boundary constraint prevents a single meshlet root.
    pub roots: Vec<NodeId>,
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Column-major view-projection matrix, matching MiniHou's camera matrices.
    pub view_projection: [[f32; 4]; 4],
    pub eye: [f32; 3],
    pub viewport_height_px: f32,
    pub near: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError { InvalidIndices, NonFinitePosition, EmptyMesh }

fn edge(a: u32, b: u32) -> (u32, u32) { (a.min(b), a.max(b)) }
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0]-b[0], a[1]-b[1], a[2]-b[2]] }
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { a[0]*b[0]+a[1]*b[1]+a[2]*b[2] }
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]] }
fn length(a: [f32; 3]) -> f32 { dot(a,a).sqrt() }

fn bounds(positions: &[[f32;3]], tris: &[Tri]) -> Sphere {
    if tris.is_empty() { return Sphere::default(); }
    let mut lo = [f32::INFINITY;3]; let mut hi = [f32::NEG_INFINITY;3];
    for tri in tris { for &v in tri { for k in 0..3 {
        lo[k] = lo[k].min(positions[v as usize][k]); hi[k] = hi[k].max(positions[v as usize][k]);
    } } }
    let center = [0,1,2].map(|k| (lo[k]+hi[k])*0.5);
    let mut radius = 0f32;
    for tri in tris { for &v in tri { radius = radius.max(length(sub(positions[v as usize], center))); } }
    Sphere { center, radius }
}

fn part1by2(mut n: u32) -> u64 {
    n &= 0x1f_ffff;
    let mut x = u64::from(n);
    x = (x | x << 32) & 0x1f00000000ffff;
    x = (x | x << 16) & 0x1f0000ff0000ff;
    x = (x | x << 8) & 0x100f00f00f00f00f;
    x = (x | x << 4) & 0x10c30c30c30c30c3;
    (x | x << 2) & 0x1249249249249249
}
fn morton(p: [f32;3], lo: [f32;3], hi: [f32;3]) -> u64 {
    let q = [0,1,2].map(|k| (((p[k]-lo[k])/(hi[k]-lo[k]).max(1e-20)).clamp(0.,1.)*2_097_151.) as u32);
    part1by2(q[0]) | part1by2(q[1]) << 1 | part1by2(q[2]) << 2
}
fn position_extent(positions: &[[f32;3]]) -> ([f32;3],[f32;3]) {
    let mut lo=[f32::INFINITY;3]; let mut hi=[f32::NEG_INFINITY;3];
    for p in positions { for k in 0..3 { lo[k]=lo[k].min(p[k]); hi[k]=hi[k].max(p[k]); } }
    (lo,hi)
}
fn sorted_tris(positions: &[[f32;3]], mut tris: Vec<Tri>, lo:[f32;3], hi:[f32;3]) -> Vec<Tri> {
    tris.sort_unstable_by_key(|tri| {
        let p=[0,1,2].map(|k| (positions[tri[0] as usize][k]+positions[tri[1] as usize][k]+positions[tri[2] as usize][k])/3.);
        morton(p,lo,hi)
    });
    tris
}

// Symmetric 4x4 plane quadric, stored as 10 upper-triangle entries.
#[derive(Clone, Copy, Default)]
struct Quadric([f64;10]);
impl Quadric {
    fn add_plane(&mut self, p: [f64;4]) {
        let mut i=0;
        for row in 0..4 { for col in row..4 { self.0[i]+=p[row]*p[col]; i+=1; } }
    }
    fn add(&mut self, other: Self) { for i in 0..10 { self.0[i]+=other.0[i]; } }
    fn cost(&self, p: [f32;3]) -> f64 {
        let v=[f64::from(p[0]),f64::from(p[1]),f64::from(p[2]),1.];
        let mut out=0.; let mut i=0;
        for row in 0..4 { for col in row..4 {
            out+=self.0[i]*v[row]*v[col]*if row==col {1.} else {2.}; i+=1;
        } }
        out.max(0.)
    }
}

/// Simplify a whole group. Every boundary vertex stays fixed, so neighbouring
/// groups at another level use exactly the same edge chain.
fn simplify_group(positions: &[[f32;3]], input: &[Tri], target: usize) -> (Vec<Tri>,f32) {
    let mut tris=input.to_vec();
    let mut total_error=0f32;
    for _pass in 0..8 {
        if tris.len()<=target { break; }
        let mut edges: HashMap<(u32,u32),u32> = HashMap::with_capacity(tris.len()*2);
        let mut quadrics: HashMap<u32,Quadric> = HashMap::with_capacity(tris.len());
        for tri in &tris {
            for (a,b) in [(tri[0],tri[1]),(tri[1],tri[2]),(tri[2],tri[0])] {
                *edges.entry(edge(a,b)).or_default()+=1;
            }
            let (a,b,c)=(positions[tri[0] as usize],positions[tri[1] as usize],positions[tri[2] as usize]);
            let n=cross(sub(b,a),sub(c,a)); let size=length(n);
            if size>1e-15 {
                let plane=[f64::from(n[0]/size),f64::from(n[1]/size),f64::from(n[2]/size),-f64::from(dot(n,a)/size)];
                for &v in tri { quadrics.entry(v).or_default().add_plane(plane); }
            }
        }
        let mut locked=HashSet::new();
        for (&(a,b),&count) in &edges { if count!=2 { locked.insert(a); locked.insert(b); } }
        let mut candidates=Vec::with_capacity(edges.len());
        for (&(a,b),&count) in &edges {
            if count!=2 || locked.contains(&a) || locked.contains(&b) { continue; }
            let mut q=quadrics[&a]; q.add(quadrics[&b]);
            let ca=q.cost(positions[a as usize]); let cb=q.cost(positions[b as usize]);
            let (keep,remove,cost)=if ca<=cb {(a,b,ca)} else {(b,a,cb)};
            candidates.push((cost,keep,remove));
        }
        candidates.sort_unstable_by(|a,b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let mut used=HashSet::new(); let mut remap=HashMap::new(); let mut pass_error=0f32;
        let wanted=(tris.len()-target).div_ceil(2);
        for &(_,keep,remove) in &candidates {
            if remap.len()>=wanted { break; }
            if used.insert(keep) && used.insert(remove) {
                remap.insert(remove,keep);
                pass_error=pass_error.max(length(sub(positions[keep as usize],positions[remove as usize])));
            }
        }
        if remap.is_empty() { break; }
        total_error+=pass_error;
        for tri in &mut tris { for v in tri { if let Some(&to)=remap.get(v) { *v=to; } } }
        tris.retain(|t| t[0]!=t[1] && t[1]!=t[2] && t[2]!=t[0]);
        // Multiple independent collapses can expose duplicate faces on unusual inputs.
        let mut unique=HashSet::with_capacity(tris.len());
        tris.retain(|t| { let mut key=*t; key.sort_unstable(); unique.insert(key) });
    }
    (tris,total_error)
}

impl ClusterLod {
    pub fn build(positions: Vec<[f32;3]>, triangles: Vec<Tri>) -> Result<Self,BuildError> {
        if triangles.is_empty() { return Err(BuildError::EmptyMesh); }
        if positions.iter().flatten().any(|x| !x.is_finite()) { return Err(BuildError::NonFinitePosition); }
        if triangles.iter().flatten().any(|&v| v as usize>=positions.len()) { return Err(BuildError::InvalidIndices); }
        let mut out=Self { positions, ..Self::default() };
        let (lo,hi)=position_extent(&out.positions);
        let ordered=sorted_tris(&out.positions,triangles,lo,hi);
        let mut frontier=Vec::new();
        for chunk in ordered.chunks(MAX_TRIANGLES) {
            let id=out.add_cluster(chunk.to_vec(),0.,0);
            let bounds=out.clusters[id as usize].bounds;
            let nid=out.nodes.len() as NodeId;
            out.nodes.push(DagNode { clusters:vec![id],children:Vec::new(),bounds,error:0.,level:0 });
            frontier.push(nid);
        }
        let mut first=true;
        while frontier.len()>1 {
            frontier.sort_unstable_by_key(|&id| {
                let p=out.nodes[id as usize].bounds.center;
                (morton(p,lo,hi),id)
            });
            let width=if first {4} else {2}; first=false;
            let mut next=Vec::new(); let mut any_reduction=false;
            for part in frontier.chunks(width) {
                if part.len()==1 { next.push(part[0]); continue; }
                let mut input=Vec::new(); let mut child_error=0f32; let mut level=0;
                for &node in part {
                    let n=&out.nodes[node as usize]; child_error=child_error.max(n.error); level=level.max(n.level+1);
                    for &cid in &n.clusters { input.extend_from_slice(&out.clusters[cid as usize].triangles); }
                }
                let (simplified,collapse)=simplify_group(&out.positions,&input,input.len().div_ceil(2));
                if simplified.len()>=input.len() { next.extend_from_slice(part); continue; }
                any_reduction=true;
                let error=child_error+collapse.max(1e-7);
                let ordered=sorted_tris(&out.positions,simplified,lo,hi);
                let ids=ordered.chunks(MAX_TRIANGLES).map(|chunk| out.add_cluster(chunk.to_vec(),error,level)).collect::<Vec<_>>();
                // Culling must retain the extent of the more detailed representation.
                let b=bounds(&out.positions,&input);
                let nid=out.nodes.len() as NodeId;
                out.nodes.push(DagNode { clusters:ids,children:part.to_vec(),bounds:b,error,level });
                next.push(nid);
            }
            if !any_reduction || next.len()==frontier.len() { out.roots=next; break; }
            frontier=next;
        }
        if out.roots.is_empty() { out.roots=frontier; }
        Ok(out)
    }

    fn add_cluster(&mut self, triangles: Vec<Tri>, error:f32, level:u32) -> ClusterId {
        let id=self.clusters.len() as ClusterId;
        let bounds=bounds(&self.positions,&triangles);
        self.clusters.push(Cluster { triangles,bounds,error,level }); id
    }

    /// Selects a group-atomic cut. `error_px` is a geometric error budget in pixels.
    pub fn select(&self, camera:Camera, error_px:f32) -> Vec<ClusterId> {
        let mut result=Vec::new(); let mut stack=self.roots.clone();
        let planes=frustum_planes(camera.view_projection);
        let pixel_scale=camera.view_projection[1][1].abs()*camera.viewport_height_px*0.5;
        while let Some(id)=stack.pop() {
            let node=&self.nodes[id as usize];
            if !visible(&planes,node.bounds) { continue; }
            let distance=(length(sub(camera.eye,node.bounds.center))-node.bounds.radius).max(camera.near.max(1e-5));
            let projected=node.error*pixel_scale/distance;
            if node.children.is_empty() || projected<=error_px {
                result.extend_from_slice(&node.clusters);
            } else { stack.extend(node.children.iter().rev().copied()); }
        }
        result
    }

    pub fn selected_triangles(&self, ids:&[ClusterId]) -> usize {
        ids.iter().map(|&id| self.clusters[id as usize].triangles.len()).sum()
    }
}

fn frustum_planes(m:[[f32;4];4])->[[f32;4];6] {
    // Clip inequalities for depth 0..w; normalize planes to compare world-space radius.
    let rows=[0,1,2,3].map(|r| [m[0][r],m[1][r],m[2][r],m[3][r]]);
    [
        [0,1,2,3].map(|i| rows[3][i]+rows[0][i]),
        [0,1,2,3].map(|i| rows[3][i]-rows[0][i]),
        [0,1,2,3].map(|i| rows[3][i]+rows[1][i]),
        [0,1,2,3].map(|i| rows[3][i]-rows[1][i]),
        rows[2],
        [0,1,2,3].map(|i| rows[3][i]-rows[2][i]),
    ]
}
fn visible(planes:&[[f32;4];6],s:Sphere)->bool {
    planes.iter().all(|p| {
        let n=[p[0],p[1],p[2]];
        dot(n,s.center)+p[3]>=-s.radius*length(n)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn grid(n:usize) -> (Vec<[f32;3]>,Vec<Tri>) {
        let mut p=Vec::with_capacity((n+1)*(n+1));
        for y in 0..=n { for x in 0..=n {
            let (u,v)=(x as f32/n as f32*1.8-0.9,y as f32/n as f32*1.8-0.9);
            p.push([u,v,(u*17.).sin()*(v*19.).cos()*0.04]);
        } }
        let mut t=Vec::with_capacity(n*n*2);
        for y in 0..n { for x in 0..n {
            let a=(y*(n+1)+x) as u32; let b=a+1; let c=a+(n+1) as u32; let d=c+1;
            t.push([a,b,d]); t.push([a,d,c]);
        } }
        (p,t)
    }
    fn sphere(lat:usize,lon:usize)->(Vec<[f32;3]>,Vec<Tri>) {
        let mut p=vec![[0.,0.,0.8]];
        for y in 1..lat { let theta=std::f32::consts::PI*y as f32/lat as f32;
            for x in 0..lon { let phi=std::f32::consts::TAU*x as f32/lon as f32;
                p.push([0.8*theta.sin()*phi.cos(),0.8*theta.sin()*phi.sin(),0.8*theta.cos()]);
            }
        }
        let south=p.len() as u32; p.push([0.,0.,-0.8]);
        let mut t=Vec::new();
        for x in 0..lon { t.push([0,1+x as u32,1+((x+1)%lon) as u32]); }
        for y in 0..lat-2 { for x in 0..lon {
            let a=1+(y*lon+x) as u32; let b=1+(y*lon+(x+1)%lon) as u32;
            let c=a+lon as u32; let d=b+lon as u32;
            t.push([a,c,b]); t.push([b,c,d]);
        } }
        for x in 0..lon { let a=1+((lat-2)*lon+x) as u32; let b=1+((lat-2)*lon+(x+1)%lon) as u32;
            t.push([a,south,b]);
        }
        (p,t)
    }
    fn camera(distance:f32)->Camera {
        Camera { view_projection:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,0.5,0.],[0.,0.,0.5,1.]],
            eye:[0.,0.,distance],viewport_height_px:900.,near:0.01 }
    }
    fn boundary(tris:impl Iterator<Item=Tri>)->HashSet<(u32,u32)> {
        let mut counts=HashMap::new();
        for t in tris { for (a,b) in [(t[0],t[1]),(t[1],t[2]),(t[2],t[0])] {
            *counts.entry(edge(a,b)).or_insert(0u32)+=1;
        } }
        assert!(counts.values().all(|&n| n<=2),"non-manifold edge in selected cut");
        counts.into_iter().filter_map(|(e,n)| (n==1).then_some(e)).collect()
    }
    fn selected(lod:&ClusterLod,camera:Camera,threshold:f32)->Vec<Tri> {
        lod.select(camera,threshold).iter().flat_map(|&id| lod.clusters[id as usize].triangles.iter().copied()).collect()
    }
    #[test]
    fn grid_cuts_keep_exact_original_boundary() {
        let (p,t)=grid(32); let original=boundary(t.iter().copied());
        let lod=ClusterLod::build(p,t).unwrap();
        for distance in [2.,20.,200.,2000.] {
            for error in [0.01,0.5,2.,10.] {
                let cut=selected(&lod,camera(distance),error);
                assert_eq!(boundary(cut.iter().copied()),original,"distance={distance} error={error}");
                assert!(cut.len()<=32*32*2);
            }
        }
    }
    #[test]
    fn sphere_cuts_remain_closed() {
        let (p,t)=sphere(24,48); assert!(boundary(t.iter().copied()).is_empty());
        let lod=ClusterLod::build(p,t).unwrap();
        for distance in [2.,8.,80.] {
            for error in [0.01,0.5,2.,10.] {
                assert!(boundary(selected(&lod,camera(distance),error).into_iter()).is_empty(),"distance={distance} error={error}");
            }
        }
    }
    #[test]
    fn error_budget_distance_and_determinism() {
        let (p,t)=grid(32); let a=ClusterLod::build(p.clone(),t.clone()).unwrap();
        let b=ClusterLod::build(p,t).unwrap();
        assert_eq!(a.roots,b.roots);
        assert_eq!(a.clusters.len(),b.clusters.len());
        for (x,y) in a.clusters.iter().zip(&b.clusters) { assert_eq!(x.triangles,y.triangles); assert_eq!(x.error,y.error); }
        for node in &a.nodes { for &child in &node.children { assert!(node.error>=a.nodes[child as usize].error); } }
        let near=a.selected_triangles(&a.select(camera(2.),1.));
        let far=a.selected_triangles(&a.select(camera(2000.),1.));
        assert!(far<near,"near={near} far={far} roots={:?} root_errors={:?}",a.roots,a.roots.iter().map(|&id|a.nodes[id as usize].error).collect::<Vec<_>>());
    }
    #[test]
    fn rejects_bad_mesh() {
        assert_eq!(ClusterLod::build(vec![[0.;3]],vec![[0,1,0]]).unwrap_err(),BuildError::InvalidIndices);
    }
    #[test]
    #[ignore = "large CPU build and timing report"]
    fn benchmark_noised_grids() {
        for n in [1000,1414,2000] {
            let (p,t)=grid(n); let start=Instant::now();
            let lod=ClusterLod::build(p,t).unwrap(); let build_ms=start.elapsed().as_secs_f64()*1000.;
            let mut levels=std::collections::BTreeMap::<u32,(usize,usize)>::new();
            for node in &lod.nodes { let e=levels.entry(node.level).or_default(); e.0+=node.clusters.len();
                e.1+=lod.selected_triangles(&node.clusters);
            }
            let mut counts=Vec::new();
            for d in [2.,200.,2000.] { let start=Instant::now(); let selected=lod.select(camera(d),2.);
                counts.push((d,lod.selected_triangles(&selected),start.elapsed().as_secs_f64()*1000.));
            }
            println!("grid {n}x{n}: source={} tris, old 10% proxy={} tris, build={build_ms:.1} ms, roots={}, clusters={}, levels={levels:?}, selected={counts:?}",2*n*n,2*n*n/10,lod.roots.len(),lod.clusters.len());
        }
    }
}
