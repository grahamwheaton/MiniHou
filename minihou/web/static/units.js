/* Blender-style display units: geometry stays in model coordinates. */
(function(root){
const metres={mm:.001,cm:.01,m:1,km:1000,in:.0254,ft:.3048};
let unit='mm',scale=1;
const api={configure(p={}){if(metres[p.length_unit])unit=p.length_unit;if(Number.isFinite(p.unit_scale)&&p.unit_scale>0)scale=p.unit_scale;if(typeof window!=='undefined')window.dispatchEvent(new Event('minihou-units-changed'));},get unit(){return unit;},get scale(){return scale;},display(v){return v*scale/metres[unit];},format(v){return Number(api.display(v).toFixed(4))+' '+unit;},parse(text){const m=String(text).trim().match(/^([+-]?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?)\s*(mm|cm|km|m|in|ft)?$/i);if(!m)throw Error('Enter a length, optionally followed by mm, cm, m, km, in or ft.');const value=Number(m[1])*metres[(m[2]||unit).toLowerCase()]/scale;if(!Number.isFinite(value))throw Error('Length must be finite.');return value;}};
root.MiniUnits=api;
})(typeof window==='undefined'?globalThis:window);
