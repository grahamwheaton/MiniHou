/* Cheap projected primitive wireframes while a shelf tool is dragged into the viewport. */
(function () {
  'use strict';
  const NS = 'http://www.w3.org/2000/svg';
  const finite3 = p => Array.isArray(p) && p.length === 3 && p.every(Number.isFinite);
  function wireframe(type, p = {}) {
    const lines = [], line = (a,b) => lines.push([a,b]);
    const ring = (radius,z,plane=2) => {
      const a=[];
      for(let i=0;i<32;i++) {
        const t=i*Math.PI/16, q=[radius*Math.cos(t),radius*Math.sin(t),z];
        a.push(plane===0?[z,q[0],q[1]]:plane===1?[q[0],z,q[1]]:q);
      }
      a.forEach((q,i)=>line(q,a[(i+1)%a.length])); return a;
    };
    if(type==='point') {
      for(let k=0;k<3;k++){const a=[0,0,0],b=[0,0,0];a[k]=-.05;b[k]=.05;line(a,b);}
    } else if(type==='box') {
      const s=p.size||[1,1,1], a=[];
      for(let i=0;i<8;i++)a.push(s.map((v,k)=>v*((i>>k)&1?0.5:-0.5)));
      for(let i=0;i<8;i++)for(let k=0;k<3;k++)if(!(i&(1<<k)))line(a[i],a[i|(1<<k)]);
    } else if(type==='sphere') {
      for(let k=0;k<3;k++)ring(p.radius??1,0,k);
    } else if(type==='circle') ring(p.radius??1,0);
    else if(type==='grid') {
      const x=(p.size_x??10)/2,y=(p.size_y??10)/2;
      const nx=Math.min(20,Math.max(2,p.columns??10)),ny=Math.min(20,Math.max(2,p.rows??10));
      for(let i=0;i<nx;i++){const a=-x+2*x*i/(nx-1);line([a,-y,0],[a,y,0]);}
      for(let i=0;i<ny;i++){const a=-y+2*y*i/(ny-1);line([-x,a,0],[x,a,0]);}
    } else if(type==='tube') {
      const h=(p.height??2)/2,a=ring(p.radius_bottom??1,-h),b=ring(p.radius_top??1,h);
      for(let i=0;i<32;i+=4)line(a[i],b[i]);
    } else return null;
    const s=p.scale||[1,1,1], u=p.uniform_scale??1, r=(p.rotate||[0,0,0]).map(v=>v*Math.PI/180);
    function transform(q) {
      let [x,y,z]=q.map((v,k)=>v*s[k]*u);
      [y,z]=[y*Math.cos(r[0])-z*Math.sin(r[0]),y*Math.sin(r[0])+z*Math.cos(r[0])];
      [x,z]=[x*Math.cos(r[1])+z*Math.sin(r[1]),-x*Math.sin(r[1])+z*Math.cos(r[1])];
      [x,y]=[x*Math.cos(r[2])-y*Math.sin(r[2]),x*Math.sin(r[2])+y*Math.cos(r[2])];
      return [x,y,z];
    }
    return lines.map(l=>l.map(transform));
  }
  window.MiniPlacementPreview = function (pv, canvas) {
    const svg=document.createElementNS(NS,'svg');
    svg.classList.add('pv-placement-preview');
    Object.assign(svg.style,{position:'absolute',pointerEvents:'none',overflow:'hidden',zIndex:'7',display:'none'});
    svg.setAttribute('aria-hidden','true');canvas.parentElement.appendChild(svg);
    const path=document.createElementNS(NS,'path');
    for(const [k,v] of Object.entries({fill:'none',stroke:'#8af0c6','stroke-width':'1.5','stroke-dasharray':'5 3','vector-effect':'non-scaling-stroke'}))path.setAttribute(k,v);
    svg.appendChild(path);
    let target=null,lines=null;
    function hide(){target=lines=null;svg.style.display='none';path.removeAttribute('d');}
    function draw(){
      if(!target||!lines)return;
      const c=canvas.getBoundingClientRect(),p=canvas.parentElement.getBoundingClientRect();
      Object.assign(svg.style,{left:(c.left-p.left)+'px',top:(c.top-p.top)+'px',width:c.width+'px',height:c.height+'px'});
      svg.setAttribute('viewBox',`0 0 ${c.width} ${c.height}`);
      const commands=[];
      for(const edge of lines){
        const q=edge.map(v=>pv.project(v.map((x,k)=>x+target[k])));
        if(q.every(v=>v&&Number.isFinite(v[0])&&Number.isFinite(v[1])))commands.push(`M${q[0][0]},${q[0][1]}L${q[1][0]},${q[1][1]}`);
      }
      path.setAttribute('d',commands.join(' '));svg.style.display=commands.length?'block':'none';
    }
    function show(type,point,params={}){
      if(!finite3(point)){hide();return false;}
      lines=wireframe(type,params);if(!lines){hide();return false;}
      target=point.slice();draw();return true;
    }
    return {show,hide,draw,active:()=>!!target,destroy:()=>{hide();svg.remove();}};
  };
  window.MiniPlacementPreview.wireframe=wireframe;
})();
