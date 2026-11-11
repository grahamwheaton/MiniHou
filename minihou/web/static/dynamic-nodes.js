/* Transient graph feedback. Never writes graph positions or schedules idle work. */
(function(root){
  'use strict';
  root.MiniDynamicNodes=function({onChange=()=>{},requestFrame=root.requestAnimationFrame?.bind(root),cancelFrame=root.cancelAnimationFrame?.bind(root),reducedMotion}={}){
    const media=root.matchMedia?.('(prefers-reduced-motion: reduce)');
    const states=new Map(); let frame=null,last=null,targets=new Map(),disposed=false;
    let settings={enabled:true,nodes:[],selected:[],dragging:[],pointer:null,paused:false};
    const reduced=()=>reducedMotion===undefined?!!media?.matches:!!reducedMotion;
    function emit(){onChange(new Map([...states].map(([id,s])=>[id,[s.x,s.y]])));}
    function reset(){if(frame!==null)cancelFrame?.(frame);frame=null;last=null;targets.clear();if(states.size){states.clear();emit();}}
    function schedule(){if(!disposed&&frame===null&&requestFrame)frame=requestFrame(t=>{frame=null;const moving=step(last===null?1/60:Math.min(.032,Math.max(.001,(t-last)/1000)));last=t;if(moving)schedule();else last=null;});}
    function step(dt=1/60){
      dt=Math.min(.032,Math.max(.001,dt));let changed=false,moving=false;
      for(const [id,s] of states){
        const goal=targets.get(id)||[0,0];
        s.vx+=(goal[0]-s.x)*100*dt;s.vy+=(goal[1]-s.y)*100*dt;
        const damping=Math.exp(-20*dt);s.vx*=damping;s.vy*=damping;s.x+=s.vx*dt;s.y+=s.vy*dt;
        const radius=Math.hypot(s.x,s.y);if(radius>12){s.x*=12/radius;s.y*=12/radius;}
        if(!targets.has(id)&&Math.hypot(s.x,s.y,s.vx,s.vy)<.025)states.delete(id);
        if(Math.hypot(s.x-goal[0],s.y-goal[1],s.vx,s.vy)>=.025)moving=true;
        changed=true;
      }
      if(changed)emit();return moving;
    }
    function update(next={}){
      if(disposed)return;settings={...settings,...next};
      const {nodes,enabled,paused,selected,dragging,pointer}=settings;
      if(!enabled||paused||reduced()||nodes.length>500){reset();return;}
      const pinned=new Set([...selected,...dragging]),ids=new Set(nodes.map(n=>n.id));let cleared=false;
      for(const id of states.keys())if(pinned.has(id)||!ids.has(id)){states.delete(id);cleared=true;}
      targets=new Map();
      if(dragging.length&&pointer?.every(Number.isFinite)){
        const nearby=[];
        for(const n of nodes){
          if(pinned.has(n.id)||!Number.isFinite(n.x)||!Number.isFinite(n.y))continue;
          const dx=n.x-pointer[0],dy=n.y-pointer[1],distance=Math.hypot(dx,dy);
          if(distance<220)nearby.push({id:n.id,dx,dy,distance});
        }
        nearby.sort((a,b)=>a.distance-b.distance);
        for(const n of nearby.slice(0,48)){
          const amount=12*(1-n.distance/220)**2,denom=n.distance||1;
          targets.set(n.id,n.distance?[amount*n.dx/denom,amount*n.dy/denom]:[0,-amount]);
          if(!states.has(n.id))states.set(n.id,{x:0,y:0,vx:0,vy:0});
        }
      }
      if(cleared)emit();
      if(states.size)schedule();
    }
    const onMedia=()=>update();media?.addEventListener?.('change',onMedia);
    return {update,step,offset:id=>{const s=states.get(id);return s?[s.x,s.y]:[0,0];},reset,
      dispose(){disposed=true;reset();media?.removeEventListener?.('change',onMedia);}};
  };
})(typeof window==='undefined'?globalThis:window);
