//! Narrow-band mesh distances and six-connected outside propagation.
//! Scalar expressions intentionally follow the numpy reference without fused multiply-add.
use std::collections::VecDeque;

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] { std::array::from_fn(|i| a[i]-b[i]) }
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 { (a[0]*b[0]+a[1]*b[1])+a[2]*b[2] }
fn cross(a:[f64;3],b:[f64;3])->[f64;3] { [a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]] }
fn vertex(t: &[f64], i:usize)->[f64;3] { [t[3*i],t[3*i+1],t[3*i+2]] }
fn closest(p:[f64;3], t:&[f64])->[f64;3] {
    let (a,b,c)=(vertex(t,0),vertex(t,1),vertex(t,2));
    let (ab,ac,ap)=(sub(b,a),sub(c,a),sub(p,a));
    let (d1,d2)=(dot(ab,ap),dot(ac,ap));
    let bp=sub(p,b); let (d3,d4)=(dot(ab,bp),dot(ac,bp));
    let cp=sub(p,c); let (d5,d6)=(dot(ab,cp),dot(ac,cp));
    let vc=d1*d4-d3*d2; let vb=d5*d2-d1*d6; let va=d3*d6-d5*d4;
    if d1<=0.0 && d2<=0.0 { return a; }
    if d3>=0.0 && d4<=d3 { return b; }
    if d6>=0.0 && d5<=d6 { return c; }
    if vc<=0.0 && d1>=0.0 && d3<=0.0 {let v=d1/(d1-d3); return std::array::from_fn(|i|a[i]+ab[i]*v);}
    if vb<=0.0 && d2>=0.0 && d6<=0.0 {let w=d2/(d2-d6); return std::array::from_fn(|i|a[i]+ac[i]*w);}
    if va<=0.0 && d4-d3>=0.0 && d5-d6>=0.0 {let w=(d4-d3)/((d4-d3)+(d5-d6)); return std::array::from_fn(|i|b[i]+(c[i]-b[i])*w);}
    let denom=1.0/(va+vb+vc); let (v,w)=(vb*denom,vc*denom);
    std::array::from_fn(|i|(a[i]+ab[i]*v)+ac[i]*w)
}
fn key(c:[usize;3],d:[usize;3])->usize {(c[0]*d[1]+c[1])*d[2]+c[2]}
fn coords(i:usize,d:[usize;3])->[usize;3] {[i/(d[1]*d[2]),i/d[2]%d[1],i%d[2]]}

/// Mark exactly those free voxels connected to any grid border.
pub fn flood(free:&[bool],dims:[usize;3])->Vec<bool> {
    let mut reached=vec![false;free.len()];
    if dims.contains(&0) {return reached;}
    let mut queue=VecDeque::new();
    for i in 0..free.len() {
        let c=coords(i,dims);
        if free[i] && (0..3).any(|a|c[a]==0 || c[a]+1==dims[a]) {reached[i]=true;queue.push_back(i);}
    }
    let strides=[dims[1]*dims[2],dims[2],1];
    while let Some(i)=queue.pop_front() {
        let c=coords(i,dims);
        for a in 0..3 { for forward in [false,true] {
            if (!forward && c[a]==0) || (forward && c[a]+1==dims[a]) {continue;}
            let j=if forward {i+strides[a]} else {i-strides[a]};
            if free[j] && !reached[j] {reached[j]=true;queue.push_back(j);}
        }}
    }
    reached
}

// Nearest corner supplies the same finite search bound as TriGrid.closest.
// The tree stores every corner (including duplicates); lower-index ties are stable.
struct Corner { p:[f64;3], index:usize, axis:usize, left:Option<usize>, right:Option<usize> }
struct Corners { nodes:Vec<Corner>, root:Option<usize> }
impl Corners {
    fn new(tris:&[f64])->Self {
        let mut points:Vec<([f64;3],usize)>=tris.chunks_exact(3).enumerate().map(|(i,p)|([p[0],p[1],p[2]],i)).collect();
        let mut tree=Self{nodes:Vec::new(),root:None}; tree.root=tree.build(&mut points,0);tree
    }
    fn build(&mut self,p:&mut [([f64;3],usize)],depth:usize)->Option<usize> {
        if p.is_empty(){return None;}
        let axis=depth%3; p.sort_unstable_by(|a,b|a.0[axis].total_cmp(&b.0[axis]).then(a.1.cmp(&b.1)));
        let mid=p.len()/2;let (point,index)=p[mid];let n=self.nodes.len();
        self.nodes.push(Corner{p:point,index,axis,left:None,right:None});
        let left=self.build(&mut p[..mid],depth+1);let right=self.build(&mut p[mid+1..],depth+1);
        self.nodes[n].left=left;self.nodes[n].right=right;Some(n)
    }
    fn query(&self,p:[f64;3])->([f64;3],f64,usize) {
        let mut best=(p,f64::INFINITY,usize::MAX);self.visit(self.root,p,&mut best);(best.0,best.1.sqrt(),best.2/3)
    }
    fn visit(&self,node:Option<usize>,p:[f64;3],best:&mut ([f64;3],f64,usize)) {
        let Some(i)=node else{return;};let n=&self.nodes[i];let v=sub(p,n.p);let d=dot(v,v);
        if d<best.1 || (d==best.1 && n.index<best.2){*best=(n.p,d,n.index);}
        let delta=p[n.axis]-n.p[n.axis];let (near,far)=if delta<0.0{(n.left,n.right)}else{(n.right,n.left)};
        self.visit(near,p,best);if delta*delta<=best.1{self.visit(far,p,best);}
    }
}

struct Grid { lo:[f64;3],hi:[f64;3],hv:[f64;3],dims:[usize;3],cells:Vec<Vec<usize>>,corners:Corners,bounds:Vec<([f64;3],[f64;3])> }
impl Grid {
    fn cell(&self,p:[f64;3])->[usize;3] {std::array::from_fn(|a|(((p[a]-self.lo[a])/self.hv[a]).floor() as isize).clamp(0,self.dims[a] as isize-1) as usize)}
    fn new(tris:&[f64])->Self {
        let mut lo=[f64::INFINITY;3]; let mut hi=[f64::NEG_INFINITY;3];
        let mut bounds=Vec::new(); let mut spans=Vec::new(); let mut area=0.0;
        for t in tris.chunks_exact(9) {
            let tl=std::array::from_fn(|a|t[a].min(t[a+3]).min(t[a+6]));
            let th=std::array::from_fn(|a|t[a].max(t[a+3]).max(t[a+6]));
            for a in 0..3 {lo[a]=lo[a].min(tl[a]);hi[a]=hi[a].max(th[a]);}
            spans.push((0..3).map(|a|th[a]-tl[a]).fold(0.0,f64::max));
            let n=cross(sub(vertex(t,1),vertex(t,0)),sub(vertex(t,2),vertex(t,0)));
            area+=dot(n,n).sqrt(); bounds.push((tl,th));
        }
        area*=0.5;
        spans.sort_by(f64::total_cmp); let pos=(spans.len()-1) as f64*0.9; let ix=pos.floor() as usize; let f=pos-ix as f64;
        // numpy's percentile lerp switches its expression at 0.5.
        let low=spans[ix]; let high=spans[(ix+1).min(spans.len()-1)];
        let percentile=if f<0.5 {low+(high-low)*f} else {high-(high-low)*(1.0-f)};
        let ext:[f64;3]=std::array::from_fn(|a|(hi[a]-lo[a]).max(1e-9));
        let big=ext.into_iter().fold(0.0,f64::max);
        let h=(2.0*area/bounds.len() as f64).sqrt().max(percentile).max(big*1e-5);
        let active=ext.map(|v|v>=0.02*big);
        let mut hv=std::array::from_fn(|a|if active[a]{h}else{ext[a]*1.0001+1e-9});
        let dims=loop {
            let dims:[usize;3]=std::array::from_fn(|a|((ext[a]/hv[a]).floor() as usize+1).min(1024));
            if dims.iter().product::<usize>()<=16*bounds.len()+1_000_000 {break dims;}
            for a in 0..3 {if active[a]{hv[a]*=1.25;}}
        };
        for a in 0..3 {hv[a]=hv[a].max(ext[a]/dims[a] as f64*1.000001);}
        let mut grid=Self{lo,hi:std::array::from_fn(|a|lo[a]+hv[a]*dims[a] as f64),hv,dims,cells:vec![Vec::new();dims.iter().product()],corners:Corners::new(tris),bounds};
        for ti in 0..grid.bounds.len() {
            let c0=grid.cell(grid.bounds[ti].0);let c1=grid.cell(grid.bounds[ti].1);
            for x in c0[0]..=c1[0] {for y in c0[1]..=c1[1] {for z in c0[2]..=c1[2] {grid.cells[key([x,y,z],dims)].push(ti);}}}
        }
        grid
    }
    fn query(&self,p:[f64;3],tris:&[f64])->([f64;3],f64,usize) {
        let qc=self.cell(p); let (mut cp,mut best,mut tri)=self.corners.query(p);
        let outside:[f64;3]=std::array::from_fn(|a|(self.lo[a]-p[a]).max(p[a]-self.hi[a]).max(0.0).powi(2));
        let total=outside[0]+outside[1]+outside[2];
        for r in 0..=*self.dims.iter().max().unwrap()+2 {
            let radius=r as isize;
            let ranges:[(isize,isize);3]=std::array::from_fn(|a|if self.dims[a]>1 {(-radius,radius)}else{(0,0)});
            let old=best;
            for x in ranges[0].0..=ranges[0].1 {for y in ranges[1].0..=ranges[1].1 {for z in ranges[2].0..=ranges[2].1 {
                let offs=[x,y,z]; if r>0 && offs.iter().map(|v|v.abs()).max()!=Some(radius){continue;}
                let c:[isize;3]=std::array::from_fn(|a|qc[a] as isize+offs[a]);
                if (0..3).any(|a|c[a]<0||c[a]>=self.dims[a] as isize){continue;}
                let blo:[f64;3]=std::array::from_fn(|a|self.lo[a]+c[a] as f64*self.hv[a]);
                let gap:[f64;3]=std::array::from_fn(|a|(blo[a]-p[a]).max(p[a]-blo[a]-self.hv[a]).max(0.0));
                if dot(gap,gap)>=old*old {continue;}
                for &ti in &self.cells[key(c.map(|v|v as usize),self.dims)] {
                    let (tl,th)=self.bounds[ti];
                    let gap=std::array::from_fn(|a|(tl[a]-p[a]).max(p[a]-th[a]).max(0.0));
                    if dot(gap,gap)>=old*old {continue;}
                    let pt=closest(p,&tris[ti*9..ti*9+9]);let delta=sub(pt,p);let dist=dot(delta,delta).sqrt();
                    if dist<best {best=dist;cp=pt;tri=ti;}
                }
            }}}
            let mut bound=f64::INFINITY;
            for a in 0..3 {if self.dims[a]>1 {
                let below=if qc[a]>r {p[a]-(self.lo[a]+(qc[a] as f64-r as f64)*self.hv[a])}else{f64::INFINITY};
                let above=if qc[a]+r<self.dims[a]-1 {self.lo[a]+(qc[a]+r+1) as f64*self.hv[a]-p[a]}else{f64::INFINITY};
                let g=below.min(above).max(0.0);bound=bound.min(g*g+(total-outside[a]));
            }}
            if best*best<=bound {break;}
        }
        (cp,best,tri)
    }
}

/// Signed distances on a caller-selected grid; triangles are a flat `(T,3,3)` buffer.
pub fn from_mesh(tris:&[f64],origin:[f64;3],dims:[usize;3],h:f64,band:f64)->Result<Vec<f32>,String> {
    if !tris.len().is_multiple_of(9) {return Err("triangles must have shape (T, 3, 3)".into());}
    if !h.is_finite() || h<=0.0 {return Err("voxel size must be positive and finite".into());}
    let size=dims.iter().try_fold(1usize,|n,&d|n.checked_mul(d)).ok_or("grid size overflow")?;
    if size>24_000_000 {return Err("too many voxels; use a larger voxel size".into());}
    let mut values=vec![band as f32;size]; if tris.is_empty() || size==0 {return Ok(values);}
    // A collapsed mesh has zero normals, hence every voxel is outside. Avoid a
    // pathological million-cell TriGrid for its artificial 1e-9 extent.
    let first=vertex(tris,0);
    if tris.chunks_exact(3).all(|p|p==first) {
        for (i,value) in values.iter_mut().enumerate() {
            let c=coords(i,dims);let p=std::array::from_fn(|a|origin[a]+c[a] as f64*h);
            let delta=sub(p,first);*value=dot(delta,delta).sqrt().min(band) as f32;
        }
        return Ok(values);
    }
    let grid=Grid::new(tris); let mut candidates=vec![false;size];let mut count=0usize;
    for &(tl,th) in &grid.bounds {
        let a:[usize;3]=std::array::from_fn(|i|(((tl[i]-band-origin[i])/h).floor() as isize).clamp(0,dims[i] as isize-1) as usize);
        let b:[usize;3]=std::array::from_fn(|i|(((th[i]+band-origin[i])/h).ceil() as isize).clamp(0,dims[i] as isize-1) as usize);
        count+=(b[0]-a[0]+1)*(b[1]-a[1]+1)*(b[2]-a[2]+1);
        if count>96_000_000 {return Err("the mesh has triangles much larger than the voxel size; use a larger voxel size".into());}
        for x in a[0]..=b[0] {for y in a[1]..=b[1] {for z in a[2]..=b[2] {candidates[key([x,y,z],dims)]=true;}}}
    }
    // numpy compares float32 unsigned against a scalar cast to float32.
    let threshold=(h*0.8660254+1e-12) as f32;
    let mut wall_sign=vec![false;size];
    for i in 0..size {if candidates[i] {
        let c=coords(i,dims);let p=std::array::from_fn(|a|origin[a]+c[a] as f64*h);
        let (cp,dist,ti)=grid.query(p,tris);values[i]=dist.min(band) as f32;
        if values[i]<threshold {
            let t=&tris[ti*9..ti*9+9];let n=cross(sub(vertex(t,1),vertex(t,0)),sub(vertex(t,2),vertex(t,0)));
            wall_sign[i]=dot(sub(p,cp),n)<0.0;
        }
    }}
    let free:Vec<bool>=values.iter().map(|&v|v>=threshold).collect();let outside=flood(&free,dims);
    for i in 0..size {if if free[i] {!outside[i]}else{wall_sign[i]} {values[i] = -values[i];}}
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn flood_enclosed() {let mut f=vec![true;125]; for i in 0..125 {let c=coords(i,[5;3]);if c.iter().all(|&v|v>0&&v<4)&&c.iter().any(|&v|v==1||v==3){f[i]=false;}}let r=flood(&f,[5;3]);assert!(!r[62]);assert!(r[0]);}
    #[test] fn empty_mesh() {assert_eq!(from_mesh(&[],[0.0;3],[2;3],1.0,3.0).unwrap(),vec![3.0;8]);}
    #[test] fn triangle() {let t=[0.,0.,0.,1.,0.,0.,0.,1.,0.];assert_eq!(closest([0.2,0.2,1.0],&t),[0.19999999999999998,0.19999999999999998,0.0]);}
}
