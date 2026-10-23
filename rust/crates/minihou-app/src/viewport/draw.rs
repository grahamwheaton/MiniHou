use super::*;

#[derive(Clone, Copy)]
enum ChunkRange { Visible, Other, All }

fn draw_chunks<'a>(pass: &mut wgpu::RenderPass<'a>, chunks: &'a [MeshChunk], use_cd: bool,
                   visibility: ObjectVisibility, range: ChunkRange) {
    for chunk in chunks {
        let indices = match range {
            ChunkRange::Visible => 0..if visibility == ObjectVisibility::ShowAll {chunk.total} else {chunk.selected},
            ChunkRange::Other => chunk.selected..chunk.total,
            ChunkRange::All => 0..chunk.total,
        };
        if indices.is_empty() {continue;}
        pass.set_vertex_buffer(0, chunk.p.slice(..));
        pass.set_vertex_buffer(1, chunk.n.slice(..));
        pass.set_vertex_buffer(2, if use_cd {chunk.cd.as_ref().unwrap_or(&chunk.colors)} else {&chunk.colors}.slice(..));
        pass.set_index_buffer(chunk.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(indices, 0, 0..1);
    }
}

fn needs_selection_depth(mode: Mode, xray: bool, editing: bool, faces: bool) -> bool {
    !xray && faces && (mode.hidden() || (editing && !mode.shaded()))
}

fn viewport_point_size(size: f32, mode: crate::ui::SelectionMode) -> f32 {
    size * if mode == crate::ui::SelectionMode::Points { 2.0 } else { 1.0 }
}

/// Selected-element orange; must match `fs_element` / `fs_face_marker` in shader.wgsl.
pub(super) const SELECTED_ELEMENT_COLOUR: [f32; 4] = [1.0, 0.627, 0.157, 1.0];
/// Near-black for unselected edit-mode points, edges and face dots (Blender parity).
pub(super) const EDIT_ELEMENT_COLOUR: [f32; 4] = [0.02, 0.02, 0.02, 1.0];

/// Colour of a point/edge element. Edit mode: black unselected, orange selected.
/// Object mode keeps the background-dependent overlay colour.
pub(super) fn element_colour(selected: bool, in_edit_mode: bool, light_background: bool) -> [f32; 4] {
    if selected { SELECTED_ELEMENT_COLOUR }
    else if in_edit_mode { EDIT_ELEMENT_COLOUR }
    else if light_background { [0.025, 0.03, 0.04, 1.] } else { [0.72, 0.75, 0.8, 1.] }
}

fn show_object_outline(selected: bool, mode: crate::ui::SelectionMode) -> bool {
    selected && mode == crate::ui::SelectionMode::Object
}

#[cfg(test)]
#[test]
fn object_outline_is_hidden_in_every_edit_selection_mode() {
    for mode in [crate::ui::SelectionMode::Points, crate::ui::SelectionMode::Edges, crate::ui::SelectionMode::Faces] {
        assert!(!show_object_outline(true, mode));
    }
    assert!(show_object_outline(true, crate::ui::SelectionMode::Object));
}

#[cfg(test)]
#[test]
fn edit_highlights_have_occluding_depth_when_faces_are_drawn() {
    assert!(needs_selection_depth(Mode::Wireframe, false, true, true));
    assert!(!needs_selection_depth(Mode::SmoothShaded, false, true, false));
    assert!(!needs_selection_depth(Mode::SmoothShaded, true, true, false));
}

#[cfg(test)]
#[test]
fn edit_mode_elements_are_black_unselected_and_orange_selected() {
    for light in [false, true] {
        let c = element_colour(false, true, light);
        assert!(c[0] < 0.05 && c[1] < 0.05 && c[2] < 0.05 && c[3] == 1.0, "{c:?}");
        assert_eq!(element_colour(true, true, light), [1.0, 0.627, 0.157, 1.0]);
    }
    assert_eq!(element_colour(false, false, false), [0.72, 0.75, 0.8, 1.]);
    assert_eq!(element_colour(false, false, true), [0.025, 0.03, 0.04, 1.]);
    let shader = include_str!("../shader.wgsl");
    assert!(shader.contains("vec4(1.,0.627,0.157,1.)") && shader.contains("vec3(1.,0.627,0.157)"));
}

#[cfg(test)]
#[test]
fn point_mode_supports_half_the_previous_marker_size() {
    assert_eq!(viewport_point_size(1.0, crate::ui::SelectionMode::Points), 2.0);
    assert_eq!(viewport_point_size(0.5, crate::ui::SelectionMode::Points), 1.0);
}

impl Gpu {
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        state: &egui_wgpu::RenderState,
        target: &Target,
        cam: &Camera,
        mode: Mode,
        overlay: Overlay,
        selection_mode: crate::ui::SelectionMode,
        selected_object: bool,
    ) {
        self.draw_raw(
            &state.device,
            &state.queue,
            target,
            cam,
            mode,
            overlay,
            selection_mode,
            selected_object,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_raw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &Target,
        cam: &Camera,
        mode: Mode,
        overlay: Overlay,
        selection_mode: crate::ui::SelectionMode,
        selected_object: bool,
    ) {
        let (view_proj,view) = cam.matrices(target.size[0] as f32/target.size[1] as f32);
        // 0 when the average edge is under about 1.5 px on screen, 1 from about 10 px (smoothstep): a dense mesh reads as shading
        let px_per_unit = target.size[1] as f32 / (2.0 * (0.8f32 / 2.0).tan() * cam.distance().max(1e-6));
        let t = ((self.edge_world * px_per_unit - 1.5) / 8.5).clamp(0.0, 1.0);
        let fade = t * t * (3.0 - 2.0 * t);
        let editing = selection_mode != crate::ui::SelectionMode::Object;
        let sketchup_clean = mode.matcap() && overlay.matcap==MatCap::SketchUpClean;
        let is_shaded_wire = sketchup_clean
            || (overlay.wireframe && mode.shaded() && !mode.bounds())
            || (editing && !mode.bounds());
        let show_wire = mode.wire() || is_shaded_wire || (overlay.xray && editing);
        let use_cd=overlay.object_color==ObjectColor::Single && self.vcd.is_some() && !mode.matcap();
        let u = Uniforms { view_proj,view,inv_view_proj:inverse_projection(view_proj),base:[0.72,0.74,0.78,1.],wire:[0.,0.,0.,if is_shaded_wire {1.} else {0.}],
            flat_shading:(mode.flat() || (mode.matcap() && overlay.matcap_flat)) as u32,wire_fade:fade,_pad:[1,if sketchup_clean {2} else {mode.matcap() as u32}],
            object_color:(overlay.object_color==ObjectColor::Random || use_cd) as u32,_pad_color:[0;3],
            floor:[10f32.powf((cam.distance().max(0.01)*0.1).log10().floor()),overlay.grid as u32 as f32,overlay.axes as u32 as f32,cam.distance().max(0.01)],
            element_size:[target.size[0] as f32,target.size[1] as f32,viewport_point_size(overlay.geometry.point_size, selection_mode),overlay.geometry.line_width],
            element_flags:[(overlay.geometry.points || selection_mode==crate::ui::SelectionMode::Points) as u32,overlay.geometry.lines as u32,points_visible(&overlay.geometry,selection_mode) as u32,(selection_mode==crate::ui::SelectionMode::Points) as u32],
            element_color:element_colour(false,editing,overlay.light_background) };
        queue.write_buffer(&self.ubuf,0,bytemuck::bytes_of(&u));
        let matcap_id=overlay.matcap.texture_id();
        if self.matcap_loaded.swap(matcap_id,std::sync::atomic::Ordering::Relaxed)!=matcap_id {
            queue.write_texture(wgpu::TexelCopyTextureInfo { texture:&self.matcap,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All },
                &overlay.matcap.pixels(128),wgpu::TexelCopyBufferLayout { offset:0,bytes_per_row:Some(128*4),rows_per_image:Some(128) },
                wgpu::Extent3d { width:128,height:128,depth_or_array_layers:1 });
        }
        // Boxes are full object bounds, never the bounds of a decimated cluster.
        let boxes=if mode.bounds() { self.boxes.as_ref() } else { None };
        let (vp,vn,vc,tri,edge)=if let Some(b)=boxes { (&b.p,&b.n,&b.c,&b.tri,&b.edge) }
            else { (&self.vp,&self.vn,if use_cd { self.vcd.as_ref().unwrap() } else { &self.vc },&self.partition_tri,&self.partition_edge) };
        let chunked = boxes.is_none() && self.mesh_source.is_some();
        let edge=if sketchup_clean { &self.sharp_edge } else { edge };
        let wire_chunks=if sketchup_clean { &self.sharp_edge_chunks } else { &self.edge_chunks };
        let visibility=overlay.visibility;
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label:Some("MiniHou viewport"),
                color_attachments:&[Some(wgpu::RenderPassColorAttachment {
                    view:&target.color,depth_slice:None,resolve_target:None,
                    ops:wgpu::Operations { load:wgpu::LoadOp::Clear(if overlay.light_background {wgpu::Color {r:0.76,g:0.78,b:0.81,a:1.}} else {wgpu::Color { r:0.244,g:0.256,b:0.278,a:1. }}),store:wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {
                    view:&target.depth,depth_ops:Some(wgpu::Operations {
                        load:wgpu::LoadOp::Clear(1.),
                        store:if show_object_outline(selected_object, selection_mode) { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard }
                    }),stencil_ops:None,
                }),
                timestamp_writes:None,occlusion_query_set:None,
            });
            pass.set_bind_group(0,&self.bind,&[]);
            pass.set_bind_group(1,&self.default_tex_bind,&[]);
            pass.set_vertex_buffer(0,vp.slice(..)); pass.set_vertex_buffer(1,vn.slice(..)); pass.set_vertex_buffer(2,vc.slice(..));
            pass.set_index_buffer(tri.buffer.slice(..),wgpu::IndexFormat::Uint32);
            let visible=tri.visible(visibility);
            if !visible.is_empty() || chunked {
                // Edit highlights need a surface depth buffer even in wireframe or
                // with face display disabled. Otherwise back-side selection bleeds through.
                if needs_selection_depth(mode, overlay.xray, editing, overlay.geometry.faces) {
                    pass.set_pipeline(&self.pipe_depth);
                    if chunked {draw_chunks(&mut pass,&self.face_chunks,use_cd,visibility,ChunkRange::Visible);}
                    else {pass.draw_indexed(visible.clone(),0,0..1);}
                }
            }
            if overlay.geometry.faces && (!visible.is_empty() || chunked) {
                if mode.shaded() || mode == Mode::WireframeGhost {
                    let textured=overlay.textures && matches!(mode,Mode::FlatShaded|Mode::SmoothShaded)
                        && overlay.object_color==ObjectColor::Single && !self.tex_batches.is_empty();
                    if textured {
                        pass.set_pipeline(if overlay.xray {&self.pipe_tex_xray}else if is_shaded_wire {&self.pipe_tex_bias}else{&self.pipe_tex});
                        for batch in &self.tex_batches {pass.set_bind_group(1,&batch.bind,&[]); pass.set_vertex_buffer(0,batch.vertex.slice(..));
                            pass.draw(0..if visibility==ObjectVisibility::ShowAll {batch.first_total}else{batch.first_selected},0..1);
                            for (buffer,selected,total) in &batch.chunks {
                                pass.set_vertex_buffer(0,buffer.slice(..));
                                pass.draw(0..if visibility==ObjectVisibility::ShowAll {*total}else{*selected},0..1);
                            }
                        }
                        pass.set_vertex_buffer(0,vp.slice(..));
                    } else {
                        pass.set_pipeline(if overlay.xray { &self.pipe_xray }
                            else if mode == Mode::WireframeGhost { &self.pipe_ghost }
                            else if is_shaded_wire { &self.pipe_surf_bias } else { &self.pipe_surf });
                        if chunked {draw_chunks(&mut pass,&self.face_chunks,use_cd,visibility,ChunkRange::Visible);}
                        else {pass.draw_indexed(visible,0,0..1);}
                    }
                }
            }
            if show_wire && overlay.geometry.lines {
                pass.set_index_buffer(edge.buffer.slice(..),wgpu::IndexFormat::Uint32);
                let visible=edge.visible(visibility);
                if mode == Mode::HiddenLineGhost && !overlay.xray {
                    pass.set_pipeline(&self.pipe_wire_hidden);
                    if chunked {draw_chunks(&mut pass,wire_chunks,use_cd,visibility,ChunkRange::Visible);}
                    else {pass.draw_indexed(visible.clone(),0,0..1);}
                }
                pass.set_pipeline(if overlay.xray || matches!(mode,Mode::Wireframe | Mode::WireframeGhost | Mode::WireBoundingBox) { &self.pipe_wire_all } else { &self.pipe_wire });
                if chunked {draw_chunks(&mut pass,wire_chunks,use_cd,visibility,ChunkRange::Visible);}
                else {pass.draw_indexed(visible,0,0..1);}
            }
            // Ghosts depth-test against selected surfaces, never write depth, and never enter the outline mask.
            if visibility == ObjectVisibility::GhostOthers {
                pass.set_index_buffer(tri.buffer.slice(..),wgpu::IndexFormat::Uint32);
                if overlay.geometry.faces { pass.set_pipeline(&self.pipe_ghost);
                    if chunked {draw_chunks(&mut pass,&self.face_chunks,use_cd,visibility,ChunkRange::Other);}
                    else {pass.draw_indexed(tri.selected..tri.total,0,0..1);}
                }
                pass.set_index_buffer(edge.buffer.slice(..),wgpu::IndexFormat::Uint32);
                if overlay.geometry.lines && show_wire { pass.set_pipeline(&self.pipe_ghost_wire);
                    if chunked {draw_chunks(&mut pass,wire_chunks,use_cd,visibility,ChunkRange::Other);}
                    else {pass.draw_indexed(edge.selected..edge.total,0,0..1);}
                }
            }
            if overlay.grid || overlay.axes {
                pass.set_pipeline(&self.pipe_floor);
                pass.draw(0..3,0..1);
            }
            if overlay.axes {
                pass.set_pipeline(&self.pipe_z_axis);pass.set_vertex_buffer(0,self.z_axis.slice(..));
                pass.draw(0..6,0..1);
            }
            if !mode.bounds() {
                pass.set_pipeline(&self.pipe_elements);
                pass.set_vertex_buffer(0,self.elements.buffer.slice(..));
                pass.draw(0..6,0..if visibility==ObjectVisibility::ShowAll {self.elements.first_total} else {self.elements.first_selected});
                for (buffer,selected,total) in &self.elements.chunks {
                    pass.set_vertex_buffer(0,buffer.slice(..));
                    pass.draw(0..6,0..if visibility==ObjectVisibility::ShowAll {*total} else {*selected});
                }
                if visibility==ObjectVisibility::GhostOthers {
                    pass.set_pipeline(&self.pipe_elements_ghost);
                    pass.set_vertex_buffer(0,self.elements.buffer.slice(..));
                    pass.draw(0..6,self.elements.first_selected..self.elements.first_total);
                    for (buffer,selected,total) in &self.elements.chunks {
                        pass.set_vertex_buffer(0,buffer.slice(..));pass.draw(0..6,*selected..*total);
                    }
                }
            }
            if self.sel_face_n > 0 && !mode.bounds() {
                pass.set_pipeline(&self.pipe_select);
                pass.set_vertex_buffer(0,self.sel_face.slice(..)); pass.set_vertex_buffer(1,self.zeros.slice(..)); pass.set_vertex_buffer(2,self.neutral_color.slice(..));
                pass.draw(0..self.sel_face_n,0..1);
            }
            if editing && self.sel_line_n > 0 && overlay.geometry.lines && !mode.bounds() {
                pass.set_pipeline(&self.pipe_bounds);
                pass.set_vertex_buffer(0,self.sel_line.slice(..)); pass.set_vertex_buffer(1,self.zeros.slice(..)); pass.set_vertex_buffer(2,self.neutral_color.slice(..));
                pass.draw(0..self.sel_line_n,0..1);
            }
            if !mode.bounds() && overlay.geometry.enabled {
                if self.group_face_n>0 {pass.set_pipeline(&self.pipe_group_face);pass.set_vertex_buffer(0,self.group_face.slice(..));pass.draw(0..self.group_face_n,0..1);}
                if self.group_line_n>0 {pass.set_pipeline(&self.pipe_group_line);pass.set_vertex_buffer(0,self.group_line.slice(..));pass.draw(0..6,0..self.group_line_n/2);}
                if self.group_point_n>0 {pass.set_pipeline(&self.pipe_group_point);pass.set_vertex_buffer(0,self.group_point.slice(..));pass.draw(0..6,0..self.group_point_n);}
                if self.barrier_face_n>0 {pass.set_pipeline(&self.pipe_barrier_face);pass.set_vertex_buffer(0,self.barrier_face.slice(..));pass.draw(0..self.barrier_face_n,0..1);}
                if self.barrier_line_n>0 {pass.set_pipeline(&self.pipe_barrier_line);pass.set_vertex_buffer(0,self.barrier_line.slice(..));pass.draw(0..self.barrier_line_n,0..1);}
                if self.target_face_n>0 {pass.set_pipeline(&self.pipe_target_face);pass.set_vertex_buffer(0,self.target_face.slice(..));pass.draw(0..self.target_face_n,0..1);}
                if self.target_line_n>0 {pass.set_pipeline(&self.pipe_target_line);pass.set_vertex_buffer(0,self.target_line.slice(..));pass.draw(0..6,0..self.target_line_n/2);}
            }
            if selection_mode == crate::ui::SelectionMode::Faces && !mode.bounds() && self.face_markers.count > 0 {
                pass.set_pipeline(if overlay.xray {&self.pipe_face_markers_xray}else{&self.pipe_face_markers});
                pass.set_vertex_buffer(0,self.face_markers.buffer.slice(..));
                pass.draw(0..6,0..self.face_markers.count);
                if self.face_markers.hover_count > 0 {
                    pass.set_vertex_buffer(0,self.face_markers.hover_buffer.slice(..));
                    pass.draw(0..6,0..self.face_markers.hover_count);
                }
            }
            if overlay.normals && self.nl_count > 0 && !mode.bounds() {
                pass.set_pipeline(&self.pipe_normals);
                pass.set_vertex_buffer(0,self.nl_p.slice(..)); pass.set_vertex_buffer(1,self.nl_n.slice(..)); pass.set_vertex_buffer(2,self.neutral_color.slice(..));
                pass.draw(0..if visibility==ObjectVisibility::ShowAll { self.nl_count } else { self.nl_selected },0..1);
            }
            if overlay.bounds && !mode.bounds() {
                pass.set_pipeline(&self.pipe_bounds);
                if let Some(b)=&self.boxes {
                    pass.set_vertex_buffer(0,b.p.slice(..)); pass.set_vertex_buffer(1,b.n.slice(..)); pass.set_vertex_buffer(2,b.c.slice(..));
                    pass.set_index_buffer(b.edge.buffer.slice(..),wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(b.edge.visible(visibility),0,0..1);
                } else {
                    if self.bb_chunks.is_empty() {
                        pass.set_vertex_buffer(0,self.bb_p.slice(..)); pass.set_vertex_buffer(1,self.bb_n.slice(..)); pass.set_vertex_buffer(2,self.neutral_color.slice(..));
                        pass.set_index_buffer(self.bb_i.slice(..),wgpu::IndexFormat::Uint32); pass.draw_indexed(0..24,0,0..1);
                    } else {
                        draw_chunks(&mut pass,&self.bb_chunks,false,visibility,ChunkRange::All);
                    }
                }
            }
        }
        let has_outline = if chunked {self.object_mask_points.is_some() && self.object_mask_n>0}
            else {boxes.is_some_and(|b| b.tri.selected>0) || self.object_mask_n > 0 || (self.object_mask.is_none() && self.ntri_idx > 0)};
        if show_object_outline(selected_object, selection_mode) && has_outline {
            {
                let mut mask_pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("MiniHou object mask"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.mask,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &target.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                mask_pass.set_bind_group(0, &self.bind, &[]);
                mask_pass.set_bind_group(1, &self.default_tex_bind, &[]);
                mask_pass.set_pipeline(&self.pipe_mask);
                let end = if boxes.is_some() { tri.selected } else if self.object_mask.is_some() { self.object_mask_n } else { self.ntri_idx };
                if end > 0 {
                    if chunked {draw_chunks(&mut mask_pass,if self.mask_chunks.is_empty() {&self.face_chunks} else {&self.mask_chunks},use_cd,visibility,ChunkRange::All);}
                    else {
                        mask_pass.set_vertex_buffer(0, vp.slice(..));
                        mask_pass.set_vertex_buffer(1, vn.slice(..));
                        mask_pass.set_vertex_buffer(2, vc.slice(..));
                        let mask_buffer=if boxes.is_some() { &tri.buffer } else { self.object_mask.as_ref().unwrap_or(&self.partition_tri.buffer) };
                        mask_pass.set_index_buffer(mask_buffer.slice(..), wgpu::IndexFormat::Uint32);
                        mask_pass.draw_indexed(0..end, 0, 0..1);
                    }
                }
            }
            {
                let outline_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("outline_bind"),
                    layout: &self.bgl_outline,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&target.mask),
                    }],
                });
                let mut outline_pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("MiniHou outline composite"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                outline_pass.set_pipeline(&self.pipe_outline);
                outline_pass.set_bind_group(0, &outline_bind, &[]);
                outline_pass.draw(0..3, 0..1);
            }
        }
        queue.submit([enc.finish()]);
    }
}
