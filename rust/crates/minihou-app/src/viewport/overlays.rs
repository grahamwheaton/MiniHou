use super::*;

impl Gpu {
    pub fn set_face_markers(&mut self, device:&wgpu::Device, scene:&Scene, topo:&crate::element_select::Topology,
        selected:&[u32], hovered:&[u32], visibility:ObjectVisibility) {
        self.face_markers.update(device,scene,topo,selected,hovered,visibility,self.object_mask_points.as_deref().unwrap_or(&[]));
    }
    pub fn select_points(&mut self, device:&wgpu::Device, scene:&Scene, points:&[u32]) {
        if self.element_points==points { return; }
        self.element_points=points.to_vec();
        self.elements=Elements::new(device,scene,self.object_mask_points.as_deref().unwrap_or(&[]),points);
    }
    /// Show the selection: `faces` = 3 positions per triangle, `lines` = 2 positions per segment (flat xyz).
    pub fn set_group_overlay(&mut self,device:&wgpu::Device,faces:&[f32],lines:&[f32]) {
        let buffer=|label,data:&[f32]|device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label),contents:if data.is_empty(){&[0u8;12]}else{bytemuck::cast_slice(data)},usage:wgpu::BufferUsages::VERTEX,
        });
        self.group_face=buffer("group faces",faces);self.group_face_n=(faces.len()/3) as u32;
        let (points, outlines)=split_point_crosses(lines);
        self.group_point=buffer("group points",&points);self.group_point_n=(points.len()/3) as u32;
        self.group_line=buffer("group outline",&outlines);self.group_line_n=(outlines.len()/3) as u32;
    }
    /// Barrier-group overlay (orange), drawn beside the purple group overlay.
    pub fn set_barrier_overlay(&mut self,device:&wgpu::Device,faces:&[f32],lines:&[f32]) {
        let buffer=|label,data:&[f32]|device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label),contents:if data.is_empty(){&[0u8;12]}else{bytemuck::cast_slice(data)},usage:wgpu::BufferUsages::VERTEX,
        });
        self.barrier_face=buffer("barrier faces",faces);self.barrier_face_n=(faces.len()/3) as u32;
        self.barrier_line=buffer("barrier outline",lines);self.barrier_line_n=(lines.len()/3) as u32;
    }
    /// Output-group target overlay (teal), shown while the target button is hovered.
    pub fn set_target_overlay(&mut self,device:&wgpu::Device,faces:&[f32],lines:&[f32]) {
        let buffer=|label,data:&[f32]|device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label),contents:if data.is_empty(){&[0u8;12]}else{bytemuck::cast_slice(data)},usage:wgpu::BufferUsages::VERTEX,
        });
        self.target_face=buffer("target faces",faces);self.target_face_n=(faces.len()/3) as u32;
        self.target_line=buffer("target outline",lines);self.target_line_n=(lines.len()/3) as u32;
    }
    pub fn set_selection(&mut self, device: &wgpu::Device, faces: &[f32], lines: &[f32]) {
        let buf = |label, data: &[f32]| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label), contents: if data.is_empty() { &[0u8; 12] } else { bytemuck::cast_slice(data) }, usage: wgpu::BufferUsages::VERTEX });
        // Highlights expand indexed faces/edges into individual vertices. Every bound
        // vertex buffer must cover that expanded count, including the constant colour.
        let need = faces.len().max(lines.len()) * 4;
        if need as u64 > self.zeros.size() { self.zeros = buf("zeros", &vec![0f32; faces.len().max(lines.len()).next_power_of_two()]); }
        if need as u64 > self.neutral_color.size() {
            self.neutral_color = buf("neutral color", &vec![0.72f32; faces.len().max(lines.len()).next_power_of_two()]);
        }
        self.sel_face = buf("sel_face", faces);
        self.sel_face_n = (faces.len() / 3) as u32;
        self.sel_line = buf("sel_line", lines);
        self.sel_line_n = (lines.len() / 3) as u32;
    }
}
