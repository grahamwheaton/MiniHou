/* Plane-based sketching. Geometry is committed through the existing curve node. */
(function(root){
'use strict';
const add=(a,b)=>a.map((v,i)=>v+b[i]),sub=(a,b)=>a.map((v,i)=>v-b[i]),mul=(a,s)=>a.map(v=>v*s),dot=(a,b)=>a.reduce((s,v,i)=>s+v*b[i],0),length=a=>Math.hypot(...a);
const planes={XY:[[1,0,0],[0,1,0],[0,0,1]],XZ:[[1,0,0],[0,0,1],[0,-1,0]],YZ:[[0,1,0],[0,0,1],[1,0,0]]};
function shape(mode,points,hover,plane='XY',segments=32){
  const p=points.slice();if(hover)p.push(hover);if(p.length<2)return p;
  const [u,v]=planes[plane],a=p[0],d=sub(p.at(-1),a);
  if(mode==='rectangle'){const x=mul(u,dot(d,u)),y=mul(v,dot(d,v));return [a,add(a,x),add(add(a,x),y),add(a,y)];}
  if(mode==='circle'){const r=length(d);segments=Math.max(3,Math.min(256,segments|0));return Array.from({length:segments},(_,i)=>add(a,add(mul(u,r*Math.cos(i*2*Math.PI/segments)),mul(v,r*Math.sin(i*2*Math.PI/segments)))));}
  return p;
}
function specification(mode,points,plane='XY',segments=32){
  const p=shape(mode,points,null,plane,segments),closed=['polygon','rectangle','circle'].includes(mode);
  if(p.length<(closed?3:2)||p.some(q=>q.length!==3||q.some(x=>!Number.isFinite(x))))throw Error('Sketch needs valid points.');
  if(p.some((q,i)=>p.slice(0,i).some(r=>length(sub(q,r))<1e-8)))throw Error('Sketch points must differ.');
  if(closed){const normal=planes[plane][2];if(p.some(q=>Math.abs(dot(sub(q,p[0]),normal))>1e-6))throw Error('Sketch face must be planar.');let area=0;const [u,v]=planes[plane];p.forEach((q,i)=>{const r=p[(i+1)%p.length];area+=dot(q,u)*dot(r,v)-dot(q,v)*dot(r,u);});if(Math.abs(area)<1e-10)throw Error('Sketch face has zero area.');
    const xy=p.map(q=>[dot(q,u),dot(q,v)]),cross=(a,b,c)=>(b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0]);
    for(let i=0;i<p.length;i++)for(let j=i+2;j<p.length;j++){if(i===0&&j===p.length-1)continue;const a=xy[i],b=xy[(i+1)%p.length],c=xy[j],d=xy[(j+1)%p.length];if(cross(a,b,c)*cross(a,b,d)<0&&cross(c,d,a)*cross(c,d,b)<0)throw Error('Sketch outline crosses itself.');}}
  return {type:'curve',params:{points:p.map(q=>q.join(',')).join(' '),closed}};
}
function infer(position,anchor,project,screen,geometry,options={}){
  const {plane='XY',origin=[0,0,0],grid=1,axis=null,snap=true}=options,[u,v,n]=planes[plane];let p=position.slice(),label='Plane';
  if(snap){let best=12;const candidate=(q,name)=>{if(options.planeOnly&&Math.abs(dot(sub(q,origin),n))>1e-5)return;const s=project(q);if(!s)return;const d=Math.hypot(s[0]-screen[0],s[1]-screen[1]);if(d<best){best=d;p=q.slice();label=name;}};
    for(const geo of (Array.isArray(geometry)?geometry:[geometry]))if(geo?.P){const P=geo.P,E=geo.edge||[];for(let i=0;i<P.length;i+=3)candidate(Array.from(P.slice(i,i+3)),'Endpoint');for(let i=0;i<E.length;i+=2)candidate([0,1,2].map(k=>(P[E[i]*3+k]+P[E[i+1]*3+k])/2),'Midpoint');}
    for(const q of options.points||[])candidate(q,'Sketch endpoint');
    if(best===12&&grid>0){p=add(origin,add(mul(u,Math.round(dot(sub(p,origin),u)/grid)*grid),mul(v,Math.round(dot(sub(p,origin),v)/grid)*grid)));label='Grid';}
    if(anchor&&best===12){for(const a of [u,v]){const q=add(anchor,mul(a,dot(sub(position,anchor),a))),s=project(q);if(s&&Math.hypot(s[0]-screen[0],s[1]-screen[1])<8){p=q;label='Axis';break;}}}
  }
  if(anchor&&axis!==null){const a=[0,0,0];a[axis]=1;p=add(anchor,mul(a,dot(sub(p,anchor),a)));label='XYZ'[axis]+' locked';}
  return {point:p,label};
}
root.MiniSketchMath={shape,specification,infer,planes};
root.MiniSketchTool=function(pv,canvas,create){
 const ns='http://www.w3.org/2000/svg',overlay=document.createElementNS(ns,'svg');Object.assign(overlay.style,{position:'absolute',inset:'0',width:'100%',height:'100%',pointerEvents:'none',overflow:'hidden'});canvas.parentElement.append(overlay);
 let mode=null,points=[],hover=null,axis=null,numeric='',busy=false,notice='',settings={plane:'XY',grid:1,segments:32,snap:true,origin:[0,0,0]},dimensions=[],lastMove=null;
 const elem=(name,attrs,text)=>{const e=document.createElementNS(ns,name);Object.entries(attrs).forEach(([k,v])=>e.setAttribute(k,v));if(text)e.textContent=text;return e;};
 function hint(){pv.showFeedback(mode?`${mode}: click points; Enter finish; Esc cancel; X/Y/Z lock; type length; Ctrl invert snap${numeric?' | '+numeric:''}${notice?' | '+notice:''}`:'');}
 function draw(){overlay.replaceChildren();const line=(ps,closed,color)=>{const projected=ps.map(p=>pv.project(p));if(projected.some(p=>!p))return;overlay.append(elem('polyline',{points:projected.map(p=>p.slice(0,2).join(',')).join(' ')+(closed&&projected.length?' '+projected[0].slice(0,2).join(','):''),fill:'none',stroke:color,'stroke-width':2}));};
 for(const d of dimensions){line(d,false,'#8ee4ff');const a=pv.project(d[0]),b=pv.project(d[1]);if(a&&b)overlay.append(elem('text',{x:(a[0]+b[0])/2+8,y:(a[1]+b[1])/2-8,fill:'#8ee4ff','font-size':13,stroke:'#17202c','stroke-width':3,'paint-order':'stroke'},length(sub(d[1],d[0])).toFixed(4)));}
 if(!mode)return;line(shape(mode,points,hover,settings.plane,settings.segments),['polygon','rectangle','circle'].includes(mode),'#ffbc60');if(hover){const p=pv.project(hover);if(p)overlay.append(elem('circle',{cx:p[0],cy:p[1],r:5,fill:'none',stroke:'#fff','stroke-width':2}));}}
 function cancel(){delete canvas.parentElement.dataset.sketchActive;mode=null;points=[];hover=null;numeric='';axis=null;notice='';canvas.style.cursor='';hint();draw();}
 async function finish(){if(busy)return;try{if(mode==='tape'){if(points.length===2)dimensions.push(points.map(p=>p.slice()));else throw Error('Pick two measurement endpoints.');cancel();return;}
 const spec=specification(mode,points,settings.plane,settings.segments);busy=true;await create(spec);cancel();}catch(e){notice=e.message;hint();}finally{busy=false;}}
 function move(e){if(!mode||busy)return;lastMove=e;const r=canvas.getBoundingClientRect(),s=[e.clientX-r.left,e.clientY-r.top],normal=planes[settings.plane][2],p=pv.planePoint(...s,settings.origin,normal);if(!p){hover=null;notice='Plane is edge-on; orbit the view';hint();return;}const hit=infer(p,points.at(-1),q=>pv.project(q),s,pv.sketchGeometry?.()||pv.geometry(),{...settings,planeOnly:['rectangle','circle','polygon'].includes(mode)&&points.length>0,axis,snap:settings.snap!==!!e.ctrlKey,points});hover=hit.point;notice=hit.label;
 const amount=Number(numeric);if(numeric&&amount>0&&points.length){const d=sub(hover,points.at(-1)),len=length(d);if(len>1e-8)hover=add(points.at(-1),mul(d,amount/len));}hint();draw();}
 canvas.addEventListener('pointermove',move,true);
 canvas.addEventListener('pointerdown',e=>{if(!mode||busy||e.button===1)return;e.preventDefault();e.stopImmediatePropagation();if(e.button===2){cancel();return;}canvas.focus({preventScroll:true});move(e);if(!hover)return;if(points.length&&length(sub(hover,points.at(-1)))<1e-8)return;if(mode==='polygon'&&points.length>=3&&length(sub(hover,points[0]))<1e-8){finish();return;}if(!points.length&&['rectangle','circle','polygon'].includes(mode))settings.origin=hover.slice();points.push(hover.slice());numeric='';if(['rectangle','circle','tape'].includes(mode)&&points.length===2)finish();else draw();},true);
 canvas.addEventListener('contextmenu',e=>{if(mode){e.preventDefault();e.stopImmediatePropagation();}},true);
 window.addEventListener('keydown',e=>{if(!mode||busy||e.target.closest?.('input,textarea,select,[contenteditable="true"]'))return;const k=e.key.toLowerCase();if(!['escape','enter','backspace','x','y','z','.'].includes(k)&&!/^\d$/.test(k))return;e.preventDefault();e.stopImmediatePropagation();if(k==='escape')cancel();else if(k==='enter'){if(numeric&&hover){points.push(hover.slice());numeric='';}finish();}else if(k==='backspace'){if(numeric)numeric=numeric.slice(0,-1);else points.pop();hint();draw();}else if('xyz'.includes(k)){const wanted='xyz'.indexOf(k);if(Math.abs(planes[settings.plane][2][wanted])>.5){notice='Axis is perpendicular to drawing plane';hint();return;}axis=axis===wanted?null:wanted;if(lastMove)move(lastMove);hint();}else{numeric+=k;if(lastMove)move(lastMove);hint();}},true);
 return {start(next,options={}){if(busy)return;if(!['line','pen','polygon','rectangle','circle','tape'].includes(next))throw Error('Unknown sketch tool');cancel();settings={...settings,...options};if(!planes[settings.plane])throw Error('Unknown drawing plane');mode=next;canvas.parentElement.dataset.sketchActive='true';canvas.tabIndex=0;canvas.focus({preventScroll:true});canvas.style.cursor='crosshair';hint();draw();},cancel,active:()=>!!mode,draw,clearDimensions(){dimensions=[];draw();},dimensions:()=>structuredClone(dimensions),panel(){const div=document.createElement('div');const label=document.createElement('p');label.textContent='Drawing plane and grid (scene units). Measurements last for this session.';div.append(label);const plane=document.createElement('select');plane.setAttribute('aria-label','Sketch plane');for(const name of Object.keys(planes)){const o=document.createElement('option');o.value=o.textContent=name;plane.append(o);}plane.value=settings.plane;plane.onchange=()=>{cancel();settings.plane=plane.value;};div.append(plane);const grid=document.createElement('input');grid.type='number';grid.min='0';grid.step='.1';grid.value=settings.grid;grid.setAttribute('aria-label','Sketch grid');grid.onchange=()=>{const n=Number(grid.value);if(Number.isFinite(n)&&n>=0)settings.grid=n;};div.append(grid);for(const name of ['line','pen','polygon','rectangle','circle','tape']){const b=document.createElement('button');b.textContent=name;b.onclick=()=>this.start(name);div.append(b);}const clear=document.createElement('button');clear.textContent='Clear measurements';clear.onclick=()=>this.clearDimensions();div.append(clear);return div;}};
};
})(typeof window==='undefined'?globalThis:window);
