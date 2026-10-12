/* Shared, deterministic geometry for viewport tools. Frames are columns (X,Y,Z). */
window.MiniInteraction = (() => {
  const add = (a,b) => a.map((v,i)=>v+b[i]), sub = (a,b)=>a.map((v,i)=>v-b[i]);
  const mul = (a,s)=>a.map(v=>v*s), dot=(a,b)=>a.reduce((s,v,i)=>s+v*b[i],0);
  const cross=(a,b)=>[a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]];
  const length=a=>Math.hypot(...a), norm=a=>mul(a,1/(length(a)||1));
  const identity=()=>[[1,0,0],[0,1,0],[0,0,1]];
  const world=(frame,v)=>v.reduce((s,x,i)=>add(s,mul(frame[i],x)),[0,0,0]);
  function frame(euler=[0,0,0]) {
    if(!Array.isArray(euler)||euler.length!==3||!euler.every(Number.isFinite))return identity();
    const [x,y,z]=euler.map(v=>v*Math.PI/180), cx=Math.cos(x),sx=Math.sin(x),cy=Math.cos(y),sy=Math.sin(y),cz=Math.cos(z),sz=Math.sin(z);
    return [[cz*cy,sz*cy,-sy],[cz*sy*sx-sz*cx,sz*sy*sx+cz*cx,cy*sx],[cz*sy*cx+sz*sx,sz*sy*cx-cz*sx,cy*cx]];
  }
  function euler(f) {
    const y=Math.asin(Math.max(-1,Math.min(1,-f[0][2]))), c=Math.cos(y);
    const x=Math.abs(c)>1e-7?Math.atan2(f[1][2],f[2][2]):Math.atan2(-f[2][1],f[1][1]);
    const z=Math.abs(c)>1e-7?Math.atan2(f[0][1],f[0][0]):0;
    return [x,y,z].map(v=>v*180/Math.PI);
  }
  function normalFrame(n,tangent) {
    if(length(n)<1e-9) return identity();
    const z=norm(n), candidate=tangent|| (Math.abs(z[0])<0.9?[1,0,0]:[0,1,0]);
    let x=sub(candidate,mul(z,dot(candidate,z)));
    if(length(x)<1e-9) x=cross(Math.abs(z[0])<0.9?[1,0,0]:[0,1,0],z);
    x=norm(x); return [x,cross(z,x),z];
  }
  function controls(value,closed=false) {
    const cs=structuredClone(typeof value==='string'?JSON.parse(value):value);
    if(!Array.isArray(cs)||cs.length<2||cs.length>10000||cs.some(c=>!['AUTO','VECTOR','ALIGNED','MIRRORED','FREE'].includes(c?.mode)||['p','left','right'].some(k=>!Array.isArray(c[k])||c[k].length!==3||!c[k].every(Number.isFinite))))throw Error('Invalid Bezier controls');
    cs.forEach((c,i)=>{
      const p=c.p, prev=(i||closed)?cs[(i-1+cs.length)%cs.length].p:p, next=(i+1<cs.length||closed)?cs[(i+1)%cs.length].p:p;
      if(c.mode==='VECTOR') {c.left=add(p,mul(sub(prev,p),1/3));c.right=add(p,mul(sub(next,p),1/3));}
      if(c.mode==='AUTO') {const d=norm(sub(next,prev));c.left=sub(p,mul(d,length(sub(p,prev))/3));c.right=add(p,mul(d,length(sub(next,p))/3));}
      if(c.mode==='ALIGNED'||c.mode==='MIRRORED') {const d=sub(c.right,p);if(length(d)>1e-12)c.left=sub(p,mul(norm(d),c.mode==='MIRRORED'?length(d):length(sub(c.left,p))));}
    }); return cs;
  }
  function moveControl(cs,index,part,position) {
    const out=structuredClone(cs),c=out[index],delta=sub(position,c[part]);
    c[part]=position.slice();
    if(part==='p') {c.left=add(c.left,delta);c.right=add(c.right,delta);}
    else {
      if(c.mode==='AUTO'||c.mode==='VECTOR')c.mode='FREE';
      const other=part==='left'?'right':'left',d=sub(position,c.p);
      if(c.mode==='ALIGNED'||c.mode==='MIRRORED')c[other]=sub(c.p,mul(norm(d),c.mode==='MIRRORED'?length(d):length(sub(cs[index][other],c.p))));
    } return out;
  }
  function split(cs,index,closed) {
    if(index===cs.length-1&&!closed)return cs;
    const out=structuredClone(cs),a=out[index],b=out[(index+1)%out.length],lerp=(x,y)=>mul(add(x,y),0.5);
    const q0=lerp(a.p,a.right),q1=lerp(a.right,b.left),q2=lerp(b.left,b.p),r0=lerp(q0,q1),r1=lerp(q1,q2);
    a.right=q0;a.mode='FREE';b.left=q2;b.mode='FREE';
    out.splice(index+1,0,{p:lerp(r0,r1),left:r0,right:r1,mode:'ALIGNED'});return out;
  }
  return {add,sub,mul,dot,cross,length,norm,identity,world,frame,euler,normalFrame,controls,moveControl,split};
})();
