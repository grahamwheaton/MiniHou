window.MiniCurveTool = (pv, canvas, save) => {
  const M=MiniInteraction, ns='http://www.w3.org/2000/svg';
  const overlay=document.createElementNS(ns,'svg');overlay.classList.add('pv-curve');canvas.parentElement.append(overlay);
  let id=null, cs=[], closed=false, selected=0, drag=null, panel=null, lastDrawing=null, defaults=null;
  const el=(tag,attrs)=>{const e=document.createElementNS(ns,tag);Object.entries(attrs).forEach(([k,v])=>e.setAttribute(k,v));return e;};
  function commit() {save(id,JSON.stringify(cs));draw();renderPanel();}
  function renderPanel() {
    if(!panel)return;panel.replaceChildren();
    if(cs.length<2){const message=document.createElement('p');message.textContent='This curve has invalid control data.';const reset=document.createElement('button');reset.textContent='Reset control points';reset.onclick=()=>{cs=M.controls(defaults,closed);selected=0;commit();};panel.append(message,reset);return;}
    const label=document.createElement('p');label.textContent=`Control point ${selected+1} of ${cs.length}`;panel.append(label);
    const mode=document.createElement('select');mode.setAttribute('aria-label','Curve handle mode');
    ['AUTO','VECTOR','ALIGNED','MIRRORED','FREE'].forEach(v=>{const o=document.createElement('option');o.value=v;o.textContent=v[0]+v.slice(1).toLowerCase();mode.append(o);});
    mode.value=cs[selected]?.mode||'AUTO';mode.onchange=()=>{cs=M.controls(cs,closed);cs[selected].mode=mode.value;cs=M.controls(cs,closed);commit();};panel.append(mode);
    const buttons=[['Append point',()=>{const p=M.add(cs.at(-1).p,[1,0,0]);cs.push({p,left:M.sub(p,[0.33,0,0]),right:M.add(p,[0.33,0,0]),mode:'AUTO'});selected=cs.length-1;}],
      ['Split segment',()=>{cs=M.split(M.controls(cs,closed),selected,closed);selected=Math.min(selected+1,cs.length-1);}],
      ['Delete point',()=>{cs.splice(selected,1);selected=Math.min(selected,cs.length-1);}]];
    buttons.forEach(([name,fn])=>{const b=document.createElement('button');b.textContent=name;b.disabled=name==='Delete point'?cs.length<=2:name==='Split segment'&&!closed&&selected===cs.length-1;b.onclick=()=>{fn();commit();};panel.append(b);});
    const p=document.createElement('p');p.className='pdoc';p.textContent='Drag anchors or tangent handles. X/Y/Z constrain; Shift for precision; Ctrl temporarily toggles snapping. Esc or right-click cancels. Split preserves the curve shape. Dragging Auto/Vector handles changes them to Free.';panel.append(p);
    const c=cs[selected];if(c) ['X','Y','Z'].forEach((axis,i)=>{const label=document.createElement('label');label.textContent=axis+' ';const input=document.createElement('input');input.type='number';input.step='0.1';input.value=c.p[i];input.setAttribute('aria-label','Control point '+axis);input.onchange=()=>{const p=c.p.slice();p[i]=Number(input.value);if(Number.isFinite(p[i])){cs=M.moveControl(cs,selected,'p',p);commit();}};label.append(input);panel.append(label);});
  }
  function draw() {
    const signature=JSON.stringify([id,selected,closed,canvas.clientWidth,canvas.clientHeight,cs,cs.map(c=>[pv.project(c.p),pv.project(c.left),pv.project(c.right)])]);
    if(signature===lastDrawing)return;lastDrawing=signature;
    overlay.setAttribute('width',canvas.clientWidth);overlay.setAttribute('height',canvas.clientHeight);overlay.replaceChildren();
    if(!id||cs.length<2)return;const resolved=M.controls(cs,closed);
    let path='';for(let i=0;i<resolved.length-(closed?0:1);i++){const a=resolved[i],b=resolved[(i+1)%resolved.length];for(let j=0;j<=32;j++){const t=j/32,u=1-t,p=[0,1,2].map(k=>u*u*u*a.p[k]+3*u*u*t*a.right[k]+3*u*t*t*b.left[k]+t*t*t*b.p[k]),q=pv.project(p);if(q)path+=(path?'L':'M')+q[0]+','+q[1];}}overlay.append(el('path',{d:path,fill:'none',stroke:'#e8b565','stroke-width':2}));
    resolved.forEach((c,i)=>{
      const p=pv.project(c.p);if(!p)return;
      if(i===selected)for(const part of ['left','right']) {const q=pv.project(c[part]);if(!q)continue;overlay.append(el('line',{x1:p[0],y1:p[1],x2:q[0],y2:q[1],stroke:'#efb657','stroke-width':1.5}));const h=el('rect',{x:q[0]-5,y:q[1]-5,width:10,height:10,fill:'#efb657',class:'curve-grab','data-point':i,'data-part':part});h.onpointerdown=e=>start(e,i,part);overlay.append(h);}
      const a=el('circle',{cx:p[0],cy:p[1],r:i===selected?7:5,fill:i===selected?'#ffb04d':'#e8edf4',stroke:'#17202c','stroke-width':2,class:'curve-grab','data-point':i,'data-part':'p'});a.onpointerdown=e=>start(e,i,'p');overlay.append(a);
    });
  }
  function start(e,i,part) {
    if(e.button!==0||drag)return;e.preventDefault();e.stopPropagation();selected=i;cs=M.controls(cs,closed);renderPanel();
    const before=structuredClone(cs),anchor=cs[i][part],r=canvas.getBoundingClientRect(),start=pv.planePoint(e.clientX-r.left,e.clientY-r.top,anchor);
    if(!start)return;let axis=null,last=e,precise=false,rawAnchor=[0,0,0],valueAnchor=[0,0,0],previousDelta=[0,0,0];
    drag={};document.body.classList.add('pv-dragging');
    function move(ev){last=ev;let p=pv.planePoint(ev.clientX-r.left,ev.clientY-r.top,anchor);if(!p)return;let d=M.sub(p,start);if(!!ev.shiftKey!==precise){precise=!!ev.shiftKey;rawAnchor=d;valueAnchor=previousDelta;}d=M.add(valueAnchor,M.mul(M.sub(d,rawAnchor),precise?0.1:1));previousDelta=d;if(axis!==null){const f=pv.orientationFrame();d=M.mul(f[axis],M.dot(d,f[axis]));}p=M.add(anchor,d);p=pv.snapPosition(p,ev.clientX-r.left,ev.clientY-r.top,ev.ctrlKey,anchor,axis);cs=M.moveControl(before,i,part,p);pv.showFeedback(`Point ${i+1} ${part==='p'?'anchor':part+' tangent'} · ${axis===null?'view plane':'axis '+'XYZ'[axis]} · ${p.map(v=>v.toFixed(3)).join(', ')} · Esc cancels`);draw();}
    function finish(cancel){window.removeEventListener('pointermove',move);window.removeEventListener('pointerup',up);window.removeEventListener('pointercancel',cancelEvent);window.removeEventListener('keydown',key,true);window.removeEventListener('contextmenu',cancelEvent,true);window.removeEventListener('pointerdown',right,true);document.body.classList.remove('pv-dragging');pv.showFeedback(null);drag=null;if(cancel)cs=before;else if(JSON.stringify(cs)!==JSON.stringify(before))save(id,JSON.stringify(cs));draw();renderPanel();}
    const right=ev=>{if(ev.button===2)cancelEvent(ev);};
    const up=()=>finish(false),cancelEvent=ev=>{ev.preventDefault();ev.stopImmediatePropagation();finish(true);};
    function key(ev){if(ev.key==='Escape'){cancelEvent(ev);return;}const k='xyz'.indexOf(ev.key.toLowerCase());if(k>=0){axis=axis===k?null:k;ev.preventDefault();ev.stopImmediatePropagation();move(last);}}
    window.addEventListener('pointermove',move);window.addEventListener('pointerup',up);window.addEventListener('pointercancel',cancelEvent);window.addEventListener('keydown',key,true);window.addEventListener('contextmenu',cancelEvent,true);window.addEventListener('pointerdown',right,true);draw();
  }
  pv.onOverlayDraw=draw;
  canvas.addEventListener('dblclick',e=>{if(!id||!e.shiftKey||drag)return;e.preventDefault();e.stopImmediatePropagation();const r=canvas.getBoundingClientRect(),p=pv.planePoint(e.clientX-r.left,e.clientY-r.top,cs.at(-1).p);if(!p)return;cs.push({p,left:M.sub(p,[0.33,0,0]),right:M.add(p,[0.33,0,0]),mode:'AUTO'});selected=cs.length-1;commit();},true);
  return {
    update(nodeId,controls,isClosed,defaultControls){if(drag)return;if(id!==nodeId)selected=0;id=nodeId;closed=!!isClosed;defaults=defaultControls;if(id){try{cs=M.controls(controls,closed);selected=Math.max(0,Math.min(selected,cs.length-1));}catch{cs=[];}}draw();},
    panel(){panel=document.createElement('div');panel.className='curve-panel';renderPanel();return panel;},
    active:()=>!!drag,
  };
};
