/* Two-stage loop cut: preview ring, choose count, slide, then commit once. */
window.MiniLoopTool=(pv,canvas,commit)=>{
 const ns='http://www.w3.org/2000/svg',overlay=document.createElementNS(ns,'svg');
 overlay.classList.add('pv-curve');overlay.style.pointerEvents='none';canvas.parentElement.append(overlay);
 let active=false,stage=0,edge=null,count=1,position=.5,startX=0;
 function draw(){overlay.replaceChildren();overlay.setAttribute('width',canvas.clientWidth);overlay.setAttribute('height',canvas.clientHeight);if(!edge)return;
 const raw=pv.geometry(),ring=pv.quadStrip(edge),key=e=>[...e].sort((a,b)=>a-b).join('-'),all=new Set();for(let i=0;i<raw.edge.length;i+=2)all.add(key([raw.edge[i],raw.edge[i+1]]));
 const faces=new Map();for(let i=0;i<raw.tri.length;i+=3){const f=raw.triPrim[i/3];if(!faces.has(f))faces.set(f,new Set());for(let j=0;j<3;j++)faces.get(f).add(raw.tri[i+j]);}
 const oriented=new Map([[key(edge),edge]]),segments=[],queue=[edge],visited=new Set();
 for(let i=0;i<queue.length;i++){const e=queue[i];for(const [f,vs]of faces){if(visited.has(f)||vs.size!==4||!vs.has(e[0])||!vs.has(e[1]))continue;const o=ring.find(o=>vs.has(o[0])&&vs.has(o[1])&&!o.some(v=>e.includes(v)));if(!o)continue;visited.add(f);const other=all.has(key([e[0],o[0]]))?o:[o[1],o[0]];segments.push([e,other]);if(!oriented.has(key(other))){oriented.set(key(other),other);queue.push(other);}}}
 const screen=(e,t)=>pv.project([0,1,2].map(k=>raw.P[e[0]*3+k]*(1-t)+raw.P[e[1]*3+k]*t));
 for(const [a,b]of segments)for(let n=0;n<count;n++){const t=count===1?position:(n+1)/(count+1),p=screen(a,t),q=screen(b,t);if(!p||!q)continue;const line=document.createElementNS(ns,'line');Object.entries({x1:p[0],y1:p[1],x2:q[0],y2:q[1],stroke:'#ffbd55','stroke-width':3}).forEach(([k,v])=>line.setAttribute(k,v));overlay.append(line);}
 pv.showFeedback(`Loop cut · ${count} cut(s) · ${stage?'move to slide; click confirms, right-click centers':'hover an edge; wheel changes count; click starts slide'} · Esc cancels`);
 }
 function move(e){if(!active)return;const r=canvas.getBoundingClientRect();if(stage){position=Math.max(.001,Math.min(.999,.5+(e.clientX-startX)/Math.max(100,canvas.clientWidth*.5)));if(e.ctrlKey)position=Math.max(.001,Math.min(.999,Math.round(position*10)/10));}else edge=pv.edgeAt(e.clientX-r.left,e.clientY-r.top);draw();}
 function finish(apply){const params=edge?{mode:'edge_loop',split_edges:`p${edge[0]}-${edge[1]}`,loops:count,position}:null;active=false;overlay.replaceChildren();pv.showFeedback(null);window.removeEventListener('pointermove',move,true);window.removeEventListener('pointerdown',down,true);window.removeEventListener('wheel',wheel,true);window.removeEventListener('keydown',key,true);if(apply&&params)commit(params);}
 function down(e){if(!active)return;e.preventDefault();e.stopImmediatePropagation();if(e.button===2){if(stage){position=.5;finish(true);}else finish(false);}else if(e.button===0&&edge){if(stage)finish(true);else{stage=1;startX=e.clientX;draw();}}}
 function wheel(e){e.preventDefault();e.stopImmediatePropagation();count=Math.max(1,Math.min(64,count+(e.deltaY<0?1:-1)));draw();}
 function key(e){if(e.key==='Escape'){e.preventDefault();e.stopImmediatePropagation();finish(false);}else if(e.key==='Enter'&&edge){e.preventDefault();e.stopImmediatePropagation();finish(true);}}
 return {active:()=>active,start(){if(active||!pv.geometry())return;active=true;stage=0;edge=null;count=1;position=.5;pv.showFeedback('Loop cut · hover a quad edge · wheel: count · Esc cancels');window.addEventListener('pointermove',move,true);window.addEventListener('pointerdown',down,true);window.addEventListener('wheel',wheel,{capture:true,passive:false});window.addEventListener('keydown',key,true);}};
};
