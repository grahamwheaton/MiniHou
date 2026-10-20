//! Shared performance budgets. Runtime code keeps its existing constants until the
//! later Project Cheetah steps connect these values to their consumers.

/// Budgets and the hardware measurements used to derive them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    pub total_ram_bytes: Option<u64>,
    pub available_ram_bytes: Option<u64>,
    pub rayon_threads: usize,
    pub max_buffer_size: Option<u64>,
    pub max_storage_buffer_binding_size: Option<u64>,
    pub cache_frames_per_node: usize,
    pub cache_frame_bytes: u64,
    pub image_in_flight_bytes: u64,
    pub image_decoded_bytes: u64,
    pub image_max_pixels: u64,
    pub material_cpu_bytes: u64,
    pub material_gpu_bytes: u64,
    pub python_output_bytes: u64,
    pub python_stderr_bytes: usize,
    pub copy_memory_bytes: u64,
    pub loop_iterations: usize,
    pub goal_solver_work: u128,
    pub goal_solver_steps: usize,
    pub goal_solver_iterations: usize,
    pub rbd_bodies: usize,
    pub rbd_collision_vertices: usize,
    pub rbd_cache_bytes: u64,
    pub rbd_substeps: usize,
    pub rbd_iterations: usize,
    pub uv_solver_iterations: usize,
    pub normal_lines: usize,
    pub rayon_chunk_size: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            total_ram_bytes: None,
            available_ram_bytes: None,
            rayon_threads: rayon::current_num_threads(),
            max_buffer_size: None,
            max_storage_buffer_binding_size: None,
            cache_frames_per_node: 2,
            cache_frame_bytes: 2 * 1024 * 1024 * 1024,
            image_in_flight_bytes: 512 * 1024 * 1024,
            image_decoded_bytes: 256 * 1024 * 1024,
            image_max_pixels: 16_000_000,
            material_cpu_bytes: 1024 * 1024 * 1024,
            material_gpu_bytes: 512 * 1024 * 1024,
            python_output_bytes: 2 * 1024 * 1024 * 1024,
            python_stderr_bytes: 64 * 1024,
            copy_memory_bytes: 256 * 1024 * 1024,
            loop_iterations: 10_000,
            goal_solver_work: 50_000_000,
            goal_solver_steps: 1000,
            goal_solver_iterations: 100,
            rbd_bodies: 2000,
            rbd_collision_vertices: 200_000,
            rbd_cache_bytes: 1024 * 1024 * 1024,
            rbd_substeps: 128,
            rbd_iterations: 500,
            uv_solver_iterations: 50_000,
            normal_lines: 60_000,
            rayon_chunk_size: 16_384,
        }
    }
}

impl Limits {
    /// Read RAM and Rayon capacity. A failed RAM query leaves the matching defaults.
    pub fn detect() -> Self {
        let mut limits = Self::default();
        let mut system = sysinfo::System::new();
        system.refresh_memory();
        let total = system.total_memory();
        let available = system.available_memory();
        if total > 0 {
            limits.total_ram_bytes = Some(total);
            limits.material_cpu_bytes = limits.material_cpu_bytes.max(total / 8);
        }
        if available > 0 {
            limits.available_ram_bytes = Some(available);
            limits.cache_frame_bytes = limits.cache_frame_bytes.max(available / 2);
            limits.copy_memory_bytes = limits.copy_memory_bytes.max(available / 4);
            limits.image_in_flight_bytes = limits.image_in_flight_bytes.max(available / 8);
            limits.image_decoded_bytes = limits.image_decoded_bytes.max(available / 16);
            limits.image_max_pixels = limits.image_max_pixels.max(limits.image_decoded_bytes / 4);
        }
        // Reference workload for the future chunked kernels. Their current RUN stays 16,384.
        limits.rayon_chunk_size = limits.rayon_chunk_size.max(
            (262_144 / limits.rayon_threads.max(1)).clamp(1024, 65_536),
        );
        limits
    }

    /// Record the device limits supplied by the viewport when that step is wired up.
    pub fn set_gpu_limits(&mut self, max_buffer_size: u64, max_storage_buffer_binding_size: u64) {
        self.max_buffer_size = Some(max_buffer_size);
        self.max_storage_buffer_binding_size = Some(max_storage_buffer_binding_size);
    }
}

#[cfg(test)]
mod tests {
    use super::Limits;

    #[test]
    fn defaults_match_existing_budgets() {
        let d = Limits::default();
        assert_eq!((d.cache_frames_per_node, d.cache_frame_bytes), (2, 2 << 30));
        assert_eq!((d.image_in_flight_bytes, d.image_decoded_bytes, d.image_max_pixels), (512 << 20, 256 << 20, 16_000_000));
        assert_eq!((d.material_cpu_bytes, d.material_gpu_bytes), (1 << 30, 512 << 20));
        assert_eq!((d.python_output_bytes, d.python_stderr_bytes), (2 << 30, 64 << 10));
        assert_eq!(d.copy_memory_bytes, 256 << 20);
        assert_eq!(d.loop_iterations, 10_000);
        assert_eq!((d.goal_solver_work, d.goal_solver_steps, d.goal_solver_iterations), (50_000_000, 1000, 100));
        assert_eq!((d.rbd_bodies, d.rbd_collision_vertices, d.rbd_cache_bytes, d.rbd_substeps, d.rbd_iterations), (2000, 200_000, 1 << 30, 128, 500));
        assert_eq!(d.uv_solver_iterations, 50_000);
        assert_eq!(d.normal_lines, 60_000);
        assert_eq!(d.rayon_chunk_size, 16_384);
        assert_eq!((d.max_buffer_size, d.max_storage_buffer_binding_size), (None, None));
    }

    #[test]
    fn detection_keeps_defaults_as_floors() {
        let d = Limits::default();
        let found = Limits::detect();
        assert!(found.rayon_threads > 0);
        assert!(found.cache_frame_bytes >= d.cache_frame_bytes);
        assert!(found.copy_memory_bytes >= d.copy_memory_bytes);
        assert!(found.image_in_flight_bytes >= d.image_in_flight_bytes);
        assert!(found.image_decoded_bytes >= d.image_decoded_bytes);
        assert!(found.image_max_pixels >= d.image_max_pixels);
        assert!(found.material_cpu_bytes >= d.material_cpu_bytes);
        assert!(found.rayon_chunk_size >= d.rayon_chunk_size);
        assert!(found.total_ram_bytes.is_none_or(|n| n > 0));
        assert!(found.available_ram_bytes.is_none_or(|n| n > 0));
        eprintln!("detected limits: {found:#?}");
    }

    #[test]
    fn gpu_limits_are_recorded() {
        let mut limits = Limits::default();
        limits.set_gpu_limits(123_456, 7_890);
        assert_eq!(limits.max_buffer_size, Some(123_456));
        assert_eq!(limits.max_storage_buffer_binding_size, Some(7_890));
    }
}
