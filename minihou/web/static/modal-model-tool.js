/* Modal scalar mesh tools; graph mutation and undo remain supplied by the editor. */
(function(root){
'use strict';
root.MiniModalModelTool=function(pv,canvas,hooks){
 let state=null,serial=0;
 const typing=e=>e.target.closest?.('input,textarea,select,[contenteditable]');
 function consume(e){e.preventDefault();e.stopImmediatePropagation();}
 function hint(s){pv.showFeedback?.(`${s.type}: ${s.typed!==''?s.typed:s.value.toFixed(3)}${s.type==='bevel'?` / ${s.divisions} divisions`:''} | mouse: 1 mm steps, type exact value, Ctrl snap, Shift precision, Enter/click confirm, Esc/right cancel`);}
 function valueOf(s){const typed=Number(s.typed);return s.typed!==''&&Number.isFinite(typed)?typed:Math.round(s.raw/(s.ctrl?.1:.001))*(s.ctrl?.1:.001);}
 function changed(s){s.value=valueOf(s);hint(s);s.pending={value:s.value,divisions:s.divisions};pump(s);}
 async function pump(s){
  if(!s.context||s.updating||s.finishing)return;s.updating=true;
  try{while(s.pending&&(!s.finishing||!s.cancelled)){const next=s.pending;s.pending=null;await hooks.update(s.context,next.value,next.divisions);}}
  catch(error){s.failure=error;s.cancelled=true;s.finishing=true;pv.showFeedback?.(error.message||String(error));}
  finally{s.updating=false;if(s.finishing)complete(s);}
 }
 async function complete(s){if(!s.context||s.updating||s.completed)return;s.completed=true;
  try{await hooks.finish(s.context,s.cancelled);}
  catch(error){pv.showFeedback?.(error.message||String(error));}
  finally{if(state===s){state=null;if(!s.failure)pv.showFeedback?.('');}}
 }
 function finish(cancel){const s=state;if(!s)return;s.cancelled=cancel;s.finishing=true;
  // A confirm must apply the latest pointer value before its graph is finalized.
  if(!cancel&&s.pending&&s.context&&!s.updating){s.finishing=false;pump(s).then(()=>{s.finishing=true;complete(s);});return;}
  if(cancel)s.pending=null;complete(s);
 }
 function move(e){const s=state;if(!s||s.finishing)return;consume(e);const dx=e.clientX-s.x,dy=e.clientY-s.y;s.x=e.clientX;s.y=e.clientY;
  s.ctrl=e.ctrlKey;const radius=Math.hypot(e.clientX-s.startX,e.clientY-s.startY),delta=['inset','bevel'].includes(s.type)?radius-s.radius:dx-dy;s.radius=radius;if(s.typed==='')s.raw+=delta*(s.scale||.01)*(e.shiftKey?.1:1);changed(s);
 }
 function down(e){if(!state)return;consume(e);if(e.button===2)finish(true);else if(e.button===0)finish(false);}
 function up(e){if(!state)return;consume(e);if(e.type==='pointercancel')finish(true);}
 function key(e){if(!state||typing(e))return;const s=state;
  if(e.key==='Escape'||e.key==='Enter'){consume(e);finish(e.key==='Escape');}
  else if(e.type==='keydown'&&!e.ctrlKey&&!e.altKey&&(/^[0-9.\-]$/.test(e.key)||e.key==='Backspace')){consume(e);s.typed=e.key==='Backspace'?s.typed.slice(0,-1):s.typed+e.key;changed(s);}
  else if(e.key==='Control'){consume(e);s.ctrl=e.type==='keydown';changed(s);}
 }
 function wheel(e){const s=state;if(!s||s.finishing)return;consume(e);if(s.type==='bevel'){const step=e.deltaY<0?1:-1;s.divisionDelta+=step;s.divisions=Math.max(1,Math.min(64,s.divisions+step));changed(s);}}
 function context(e){if(!state)return;consume(e);finish(true);}
 const listeners=[[window,'pointermove',move],[canvas,'pointerdown',down],[window,'pointerup',up],[window,'pointercancel',up],[window,'keydown',key],[window,'keyup',key],[canvas,'wheel',wheel],[canvas,'contextmenu',context]];
 listeners.forEach(([el,event,fn])=>el.addEventListener(event,fn,{capture:true,passive:false}));
 return {async start(type,x,y){if(state||!['bevel','inset','extrude','pushpull'].includes(type))return false;
   const r=canvas.getBoundingClientRect(),s={id:++serial,type,x:x??r.left+r.width/2,y:y??r.top+r.height/2,startX:x??r.left+r.width/2,startY:y??r.top+r.height/2,radius:0,typed:'',raw:0,value:0,divisions:1,divisionDelta:0,scale:.01,ctrl:false,context:null,updating:false,finishing:false,completed:false,pending:null};state=s;canvas.focus();hint(s);
   try{s.context=await hooks.start(type);if(!s.context){if(state===s)state=null;pv.showFeedback?.('');return false;}
    const base=Number(s.context.value)||0;s.raw=s.raw*((Number(s.context.scale)||.01)/s.scale)+base;s.value=s.raw;s.divisions=Math.max(1,Math.min(64,(Number(s.context.divisions)||1)+s.divisionDelta));s.scale=Number(s.context.scale)||.01;
    if(s.finishing){if(!s.cancelled&&s.pending){s.pending={value:valueOf(s),divisions:s.divisions};s.finishing=false;await pump(s);s.finishing=true;}complete(s);}else{changed(s);}return true;
   }catch(error){s.failure=error;s.cancelled=true;s.finishing=true;if(s.context)complete(s);else if(state===s)state=null;pv.showFeedback?.(error.message||String(error));return false;}
  },active:()=>state?.type||null,cancel:()=>finish(true),destroy(){finish(true);listeners.forEach(([el,event,fn])=>el.removeEventListener(event,fn,true));}};
};
})(typeof window!=='undefined'?window:globalThis);


