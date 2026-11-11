/* Viewport region gestures and geometric route preference. */
(function(root){
'use strict';
const sub=(a,b)=>a.map((x,i)=>x-b[i]),dot=(a,b)=>a.reduce((s,x,i)=>s+x*b[i],0),len=a=>Math.hypot(...a);
function contains(region,x,y){
 if(region.type==='circle')return Math.hypot(x-region.center[0],y-region.center[1])<=region.radius;
 const p=region.points||[];let inside=false;
 for(let i=0,j=p.length-1;i<p.length;j=i++){
  const a=p[j],b=p[i],dx=b[0]-a[0],dy=b[1]-a[1],l=dx*dx+dy*dy,t=l?Math.max(0,Math.min(1,((x-a[0])*dx+(y-a[1])*dy)/l)):0;
  if(Math.hypot(x-a[0]-t*dx,y-a[1]-t*dy)<1e-7)return true;
  if((a[1]>y)!==(b[1]>y)&&x<(b[0]-a[0])*(y-a[1])/(b[1]-a[1])+a[0])inside=!inside;
 }return inside;
}
// Position is a world-space point or element centre; adjacency contains element values.
// Penalize length, distance from the endpoint chord, and bends. State includes the
// incoming element so angular costs are valid. This is a preference, not edge-loop inference.
function directPath({adjacency,key,position,start,target}){
 const a=position(start),b=position(target),chord=sub(b,a),scale=len(chord)||1;
 const distance=p=>{const v=sub(p,a),t=Math.max(0,Math.min(1,dot(v,chord)/(scale*scale)));return len(sub(v,chord.map(x=>x*t)))/scale;};
 const open=[{value:start,prev:null,cost:0,path:[start]}],best=new Map();
 while(open.length){open.sort((x,y)=>x.cost-y.cost);const s=open.shift(),id=JSON.stringify([s.prev===null?null:key(s.prev),key(s.value)]);
  if(s.cost>(best.get(id)??Infinity))continue;
  if(key(s.value)===key(target))return s.path;
  for(const v of adjacency.get(key(s.value))||[]){if(s.prev!==null&&key(v)===key(s.prev))continue;
   const p=position(s.value),q=position(v),d=sub(q,p),L=len(d),incoming=s.prev===null?chord:sub(p,position(s.prev)),I=len(incoming);
   const turn=I&&L?1-Math.max(-1,Math.min(1,dot(incoming,d)/(I*L))):0;
   const cost=s.cost+Math.max(L/scale,1e-9)*(1+8*(distance(p)+distance(q))/2)+.75*turn;
   const next=JSON.stringify([key(s.value),key(v)]);if(cost<(best.get(next)??Infinity)-1e-12){best.set(next,cost);open.push({value:v,prev:s.value,cost,path:s.path.concat([v])});}
  }
 }return [];
}
root.MiniSelectionMath={contains,directPath};
root.MiniSelectionTool=function(pv,canvas){
 const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');Object.assign(svg.style,{position:'absolute',inset:'0',width:'100%',height:'100%',pointerEvents:'none'});canvas.parentElement.append(svg);
 let mode=null,drag=null,radius=30,hover=[0,0],snapshot=null;
 const typing=e=>e.target.closest?.('input,textarea,select,[contenteditable]');
 const xy=e=>{const r=canvas.getBoundingClientRect();return [e.clientX-r.left,e.clientY-r.top];};
 const feedback=()=>pv.showFeedback?.(mode?`${mode} select: left add, middle/Ctrl subtract; Esc cancel${mode==='circle'?'; wheel radius; Enter finish':''}`:'');
 function draw(){svg.replaceChildren();if(!mode)return;const shape=document.createElementNS(svg.namespaceURI,mode==='circle'?'circle':mode==='box'?'rect':'polyline');
  shape.setAttribute('fill',mode==='circle'?'none':'#7ebcff18');shape.setAttribute('stroke','#8fc7ff');shape.setAttribute('stroke-width','1.5');
  if(mode==='circle'){shape.setAttribute('cx',hover[0]);shape.setAttribute('cy',hover[1]);shape.setAttribute('r',radius);}
  else if(drag&&mode==='box'){shape.setAttribute('x',Math.min(drag.start[0],hover[0]));shape.setAttribute('y',Math.min(drag.start[1],hover[1]));shape.setAttribute('width',Math.abs(hover[0]-drag.start[0]));shape.setAttribute('height',Math.abs(hover[1]-drag.start[1]));}
  else if(drag)shape.setAttribute('points',drag.points.map(p=>p.join(',')).join(' '));svg.append(shape);
 }
 function finish(rollback=false){if(rollback&&snapshot)pv.restoreSelection?.(snapshot);mode=null;drag=null;snapshot=null;draw();feedback();}
 function paint(p){pv.regionSelect({type:'circle',center:p,radius},drag.how);}
 function down(e){if(!mode||!canvas.contains(e.target))return;e.preventDefault();e.stopImmediatePropagation();if(e.button===2){finish(true);return;}if(e.button!==0&&e.button!==1)return;
  hover=xy(e);drag={start:hover,points:[hover],how:e.button===1||e.ctrlKey?'subtract':e.shiftKey||mode==='circle'?'add':'replace',id:e.pointerId};canvas.setPointerCapture?.(e.pointerId);if(mode==='circle')paint(hover);draw();}
 function move(e){if(!mode)return;hover=xy(e);if(drag){if(mode==='circle'){const last=drag.points.at(-1),d=Math.hypot(hover[0]-last[0],hover[1]-last[1]),n=Math.max(1,Math.ceil(d/Math.max(1,radius/2)));for(let i=1;i<=n;i++)paint([last[0]+(hover[0]-last[0])*i/n,last[1]+(hover[1]-last[1])*i/n]);}
  if(mode!=='box')drag.points.push(hover);}draw();}
 function up(e){if(!mode||!drag)return;e.preventDefault();e.stopImmediatePropagation();if(e.type==='pointercancel'){finish(true);return;}hover=xy(e);
  if(mode==='box')pv.boxSelect(...drag.start,...hover,drag.how);else if(mode==='lasso'){drag.points.push(hover);if(drag.points.length>=3)pv.regionSelect({type:'lasso',points:drag.points},drag.how);}drag=null;if(mode!=='circle')finish();else draw();}
 function keydown(e){if(!mode||typing(e))return;if(e.key==='Escape'||e.key==='Enter'){e.preventDefault();e.stopImmediatePropagation();finish(e.key==='Escape');}}
 function wheel(e){if(mode!=='circle'||!canvas.contains(e.target))return;e.preventDefault();e.stopImmediatePropagation();radius=Math.max(4,Math.min(300,radius+(e.deltaY<0?5:-5)));draw();}
 function context(e){if(!mode)return;e.preventDefault();e.stopImmediatePropagation();finish(true);}
 const listeners=[[canvas,'pointerdown',down],[window,'pointermove',move],[window,'pointerup',up],[window,'pointercancel',up],[window,'keydown',keydown],[canvas,'wheel',wheel],[canvas,'contextmenu',context]];
 listeners.forEach(([el,event,fn])=>el.addEventListener(event,fn,{capture:true,passive:false}));
 return {start(kind){if(!['box','circle','lasso'].includes(kind)||pv.mode==='view'||(pv.canPick&&!pv.canPick()))return false;if(mode)finish(true);mode=kind;snapshot=pv.selection();canvas.focus();feedback();draw();return true;},active:()=>mode,cancel:()=>finish(true),radius:()=>radius,destroy(){finish(true);listeners.forEach(([el,event,fn])=>el.removeEventListener(event,fn,true));svg.remove();}};
};
})(typeof window!=='undefined'?window:globalThis);
