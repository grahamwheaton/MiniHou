/* Explicit length semantics: never infer units from generic float/vector values. */
(function(root){
'use strict';
const fields={
 group_create:['box_center','box_size','min_edge_length','max_edge_length'], group_transfer:['distance_threshold'], group_by_proximity:['distance','position'],
 point:['center'], box:['center','size'], sphere:['center','radius'], grid:['center','size_x','size_y'],
 tube:['center','radius_bottom','radius_top','height'], torus:['center','radius','thickness'], circle:['center','radius'],
 line:['origin','length'], helix:['radius','radius_end','height'],
 transform:['translate','pivot'], soft_transform:['translate','center','radius'],
 polybevel:['distance'], polyextrude:['distance','inset'], inset:['amount'], outset:['amount'], pushpull:['distance'],
 match_size:['size','center'], lattice_grid:['size','center'], mirror:['origin','distance'], clip:['origin'],
 fuse:['distance'], resample:['length'], fillet:['radius'], sweep:['radius'], curve_simplify:['tolerance'],
};
root.MiniLengthParams={has:(type,param)=>(fields[type]||[]).includes(param),fields};
})(typeof window!=='undefined'?window:globalThis);
