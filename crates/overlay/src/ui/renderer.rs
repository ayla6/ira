use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

use ash::vk;

use crate::types::DeviceFns;

use super::atlas;
use super::model;
use super::resources;
use super::text;
use super::vertex::{DrawCmd, PushConstants, Vertex};
use super::widget::{DrawCtx, Event, EventCtx, LayoutCtx, Padding, Rect, Widget};
use super::widgets::{Button, Column, Label, Panel, Row};

const MAX_VERTICES: usize = 100_000;
const MAX_INDICES: usize = 200_000;

struct FrameData {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    draw_cmds: Vec<DrawCmd>,
}

static FRAME_DATA: Mutex<Option<FrameData>> = Mutex::new(None);
/// Frame buffers reused across frames to avoid per-frame allocations.
static FRAME_VERTICES: Mutex<Vec<Vertex>> = Mutex::new(Vec::new());
static FRAME_INDICES: Mutex<Vec<u32>> = Mutex::new(Vec::new());
static FRAME_CMDS: Mutex<Vec<DrawCmd>> = Mutex::new(Vec::new());

static UI_TREE: Mutex<Option<Box<dyn Widget>>> = Mutex::new(None);
static FOCUSED_INDEX: AtomicUsize = AtomicUsize::new(0);
static MOUSE_DOWN_INDEX: AtomicUsize = AtomicUsize::new(usize::MAX);

static REBUILD_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Forces a UI rebuild on the next `prepare()` call.
static UI_DIRTY: AtomicBool = AtomicBool::new(true);

/// Marks the UI as dirty so it will be rebuilt on the next frame.
pub fn mark_ui_dirty() {
    UI_DIRTY.store(true, Ordering::Relaxed);
}

pub struct UiRenderer {
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    pipeline: vk::Pipeline,
    pipeline_layout: vk::PipelineLayout,
    set_layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    atlas_image: vk::Image,
    atlas_memory: vk::DeviceMemory,
    atlas_view: vk::ImageView,
    sampler: vk::Sampler,
    shader_vert: vk::ShaderModule,
    shader_frag: vk::ShaderModule,
    vertex_buffer: vk::Buffer,
    vertex_memory: vk::DeviceMemory,
    vertex_ptr: *mut std::ffi::c_void,
    index_buffer: vk::Buffer,
    index_memory: vk::DeviceMemory,
    index_ptr: *mut std::ffi::c_void,
}

unsafe impl Send for UiRenderer {}
unsafe impl Sync for UiRenderer {}
impl Clone for UiRenderer {
    fn clone(&self) -> Self {
        *self
    }
}
impl Copy for UiRenderer {}

impl UiRenderer {
    /// # Safety
    /// Caller must ensure `fns` contains valid function pointers for `device`,
    /// and that `cmd_pool` and `render_pass` are valid for `device`.
    pub unsafe fn new(
        fns: DeviceFns,
        device: vk::Device,
        physical_device: vk::PhysicalDevice,
        cmd_pool: vk::CommandPool,
        render_pass: vk::RenderPass,
    ) -> Option<Self> {
        let (set_layout, pool, set) = resources::create_descriptors(fns, device)?;
        let sampler = resources::create_sampler(fns, device);
        let (atlas_image, atlas_memory, atlas_view) =
            resources::create_atlas_texture(fns, device, physical_device, cmd_pool)?;

        let image_info = vk::DescriptorImageInfo::default()
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image_view(atlas_view)
            .sampler(sampler);
        let write = vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(std::slice::from_ref(&image_info));
        (fns.update_descriptor_sets)(device, 1, &write, 0, std::ptr::null());

        let (pipeline_layout, pipeline, shader_vert, shader_frag) =
            resources::create_pipeline(fns, device, render_pass, set_layout);

        let (vertex_buffer, vertex_memory, vertex_ptr) = resources::create_vertex_buffer(
            fns,
            device,
            physical_device,
            (MAX_VERTICES * 20) as u64,
        )?;
        let (index_buffer, index_memory, index_ptr) =
            resources::create_index_buffer(fns, device, physical_device, (MAX_INDICES * 4) as u64)?;

        text::init_fonts();
        atlas::clear_cache();
        text::clear_cache();
        *UI_TREE.lock().unwrap() = model::build_ui().or_else(|| Some(build_fallback_ui()));

        Some(Self {
            device,
            physical_device,
            pipeline,
            pipeline_layout,
            set_layout,
            pool,
            set,
            atlas_image,
            atlas_memory,
            atlas_view,
            sampler,
            shader_vert,
            shader_frag,
            vertex_buffer,
            vertex_memory,
            vertex_ptr,
            index_buffer,
            index_memory,
            index_ptr,
        })
    }

    /// # Safety
    /// `fns` must be valid for `self.device`. `cmd` must be in the recording state.
    /// `fence` must be the fence that will be signaled after the command buffer
    /// containing the copy is submitted.
    pub unsafe fn update_atlas(&self, fns: DeviceFns, cmd: vk::CommandBuffer, fence: vk::Fence) {
        let uploads = atlas::take_pending_uploads();
        if uploads.is_empty() {
            return;
        }

        let total_size: u64 = uploads.iter().map(|u| u.pixels.len() as u64).sum();
        let Some((staging_buf, staging_ptr, _capacity)) =
            atlas::prepare_staging(fns, self.device, self.physical_device, total_size)
        else {
            return;
        };

        let mut offset = 0u64;
        let mut regions = Vec::with_capacity(uploads.len());
        for u in &uploads {
            std::ptr::copy_nonoverlapping(
                u.pixels.as_ptr(),
                staging_ptr.add(offset as usize),
                u.pixels.len(),
            );
            regions.push(
                vk::BufferImageCopy::default()
                    .buffer_offset(offset)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_offset(vk::Offset3D {
                        x: u.atlas_x as i32,
                        y: u.atlas_y as i32,
                        z: 0,
                    })
                    .image_extent(vk::Extent3D {
                        width: u.width,
                        height: u.height,
                        depth: 1,
                    }),
            );
            offset += u.pixels.len() as u64;
        }

        let subresource = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        let barrier_to_dst = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_access_mask(vk::AccessFlags::SHADER_READ)
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.atlas_image)
            .subresource_range(subresource);
        (fns.cmd_pipeline_barrier)(
            cmd,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            1,
            &barrier_to_dst,
        );

        (fns.cmd_copy_buffer_to_image)(
            cmd,
            staging_buf,
            self.atlas_image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            regions.len() as u32,
            regions.as_ptr(),
        );

        let barrier_to_read = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.atlas_image)
            .subresource_range(subresource);
        (fns.cmd_pipeline_barrier)(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            1,
            &barrier_to_read,
        );

        atlas::set_staging_fence(fence);
    }

    pub fn prepare(&self, extent: vk::Extent2D) {
        let screen_w = extent.width as f32;
        let screen_h = extent.height as f32;

        super::set_screen_size(extent.width, extent.height);

        // Rebuild the UI tree when dirty (first run or SHM data changed).
        if UI_DIRTY.swap(false, Ordering::Relaxed) {
            if let Some(new_tree) = model::build_ui() {
                *UI_TREE.lock().unwrap() = Some(new_tree);
                model::reset_change_tracker();
            }
        }

        // Periodically check for SHM changes (every ~1s at 60fps).
        let count = REBUILD_COUNTER.fetch_add(1, Ordering::Relaxed);
        if count.is_multiple_of(60) && model::check_shm_changed().unwrap_or(false) {
            UI_DIRTY.store(true, Ordering::Relaxed);
        }

        let mut tree_guard = UI_TREE.lock().unwrap();
        let Some(tree) = tree_guard.as_mut() else {
            return;
        };

        let ctx = LayoutCtx;
        let measured = tree.measure(&ctx);
        tree.layout(
            &ctx,
            Rect {
                x: 10.0,
                y: 10.0,
                width: measured.width,
                height: measured.height,
            },
        );

        let mut focus_bounds = Vec::new();
        tree.collect_focusable(&mut focus_bounds);

        let events = super::take_events();
        let mut focused = FOCUSED_INDEX.load(Ordering::Relaxed);

        for event in &events {
            match event {
                Event::NavUp | Event::NavDown | Event::NavLeft | Event::NavRight => {
                    if !focus_bounds.is_empty() && focused < focus_bounds.len() {
                        let current = focus_bounds[focused];
                        if let Some(idx) = super::focus::navigate(*event, current, &focus_bounds) {
                            focused = idx;
                        }
                    }
                }
                Event::Activate => {
                    let event_ctx = EventCtx {
                        focused_index: Some(focused),
                    };
                    tree.handle_event(&event_ctx, event);
                }
                Event::MouseMove { x, y } => {
                    if let Some(idx) = hit_test(&focus_bounds, *x, *y) {
                        focused = idx;
                    }
                }
                Event::MouseDown { x, y } => {
                    if let Some(idx) = hit_test(&focus_bounds, *x, *y) {
                        focused = idx;
                        MOUSE_DOWN_INDEX.store(idx, Ordering::Relaxed);
                    }
                }
                Event::MouseUp { x, y } => {
                    let down_idx = MOUSE_DOWN_INDEX.swap(usize::MAX, Ordering::Relaxed);
                    if let Some(idx) = hit_test(&focus_bounds, *x, *y) {
                        if idx == down_idx {
                            let event_ctx = EventCtx {
                                focused_index: Some(idx),
                            };
                            tree.handle_event(&event_ctx, &Event::Activate);
                        }
                    }
                }
                Event::Scroll { .. } => {
                    let event_ctx = EventCtx {
                        focused_index: Some(focused),
                    };
                    tree.handle_event(&event_ctx, event);
                }
            }
        }

        FOCUSED_INDEX.store(focused, Ordering::Relaxed);

        let mut vertices = FRAME_VERTICES.lock().unwrap();
        let mut indices = FRAME_INDICES.lock().unwrap();
        let mut draw_cmds = FRAME_CMDS.lock().unwrap();
        vertices.clear();
        indices.clear();
        draw_cmds.clear();
        {
            let mut draw_ctx = DrawCtx {
                vertices: &mut vertices,
                indices: &mut indices,
                draw_cmds: &mut draw_cmds,
                focused_index: Some(focused),
                clip_x: 0.0,
                clip_y: 0.0,
                clip_w: screen_w,
                clip_h: screen_h,
            };
            tree.draw(&mut draw_ctx);
        }

        *FRAME_DATA.lock().unwrap() = Some(FrameData {
            vertices: std::mem::take(&mut *vertices),
            indices: std::mem::take(&mut *indices),
            draw_cmds: std::mem::take(&mut *draw_cmds),
        });
    }

    /// # Safety
    /// `fns` must be valid for `self.device`. `cmd` must be in the recording state
    /// and inside an active render pass.
    pub unsafe fn draw(&self, fns: DeviceFns, cmd: vk::CommandBuffer, extent: vk::Extent2D) {
        let frame = FRAME_DATA.lock().unwrap().take();
        let Some(frame) = frame else { return };
        let FrameData {
            mut vertices,
            mut indices,
            draw_cmds,
        } = frame;

        let v_bytes = vertices.len() * 20;
        let i_bytes = indices.len() * 4;
        if v_bytes > MAX_VERTICES * 20 || i_bytes > MAX_INDICES * 4 {
            return;
        }
        if v_bytes > 0 {
            std::ptr::copy_nonoverlapping(
                vertices.as_ptr() as *const u8,
                self.vertex_ptr as *mut u8,
                v_bytes,
            );
        }
        if i_bytes > 0 {
            std::ptr::copy_nonoverlapping(
                indices.as_ptr() as *const u8,
                self.index_ptr as *mut u8,
                i_bytes,
            );
        }

        (fns.cmd_bind_pipeline)(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
        let vb = [self.vertex_buffer];
        let offsets = [0u64];
        (fns.cmd_bind_vertex_buffers)(cmd, 0, 1, vb.as_ptr(), offsets.as_ptr());
        (fns.cmd_bind_index_buffer)(cmd, self.index_buffer, 0, vk::IndexType::UINT32);
        (fns.cmd_bind_descriptor_sets)(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            self.pipeline_layout,
            0,
            1,
            &self.set,
            0,
            std::ptr::null(),
        );

        let screen_w = extent.width as f32;
        let screen_h = extent.height as f32;

        // The pipeline uses dynamic viewport state; without an explicit
        // vkCmdSetViewport every draw is clipped to undefined state.
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: screen_w,
            height: screen_h,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        (fns.cmd_set_viewport)(cmd, 0, 1, &viewport as *const vk::Viewport);

        for dc in &draw_cmds {
            let pc = PushConstants {
                screen_size: [screen_w, screen_h],
                shape_size: dc.shape_size,
                corner_radius: dc.corner_radius,
                is_shape: dc.draw_mode,
            };
            (fns.cmd_push_constants)(
                cmd,
                self.pipeline_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                24,
                &pc as *const PushConstants as *const std::ffi::c_void,
            );
            let scissor = vk::Rect2D {
                offset: vk::Offset2D {
                    x: dc.clip_x.max(0.0) as i32,
                    y: dc.clip_y.max(0.0) as i32,
                },
                extent: vk::Extent2D {
                    width: dc.clip_w.max(0.0) as u32,
                    height: dc.clip_h.max(0.0) as u32,
                },
            };
            (fns.cmd_set_scissor)(cmd, 0, 1, &scissor as *const vk::Rect2D);
            (fns.cmd_draw_indexed)(cmd, dc.index_count, 1, dc.index_offset, dc.vertex_offset, 0);
        }

        vertices.clear();
        indices.clear();
        *FRAME_VERTICES.lock().unwrap() = vertices;
        *FRAME_INDICES.lock().unwrap() = indices;
        *FRAME_CMDS.lock().unwrap() = draw_cmds;
    }

    /// # Safety
    /// `fns` must be valid for `self.device`. No other references to GPU
    /// resources may exist when this is called.
    pub unsafe fn destroy(&self, fns: DeviceFns) {
        let d = self.device;
        (fns.destroy_pipeline)(d, self.pipeline, std::ptr::null());
        (fns.destroy_pipeline_layout)(d, self.pipeline_layout, std::ptr::null());
        (fns.destroy_descriptor_set_layout)(d, self.set_layout, std::ptr::null());
        (fns.destroy_descriptor_pool)(d, self.pool, std::ptr::null());
        (fns.destroy_sampler)(d, self.sampler, std::ptr::null());
        (fns.destroy_image_view)(d, self.atlas_view, std::ptr::null());
        (fns.destroy_image)(d, self.atlas_image, std::ptr::null());
        (fns.free_memory)(d, self.atlas_memory, std::ptr::null());
        (fns.destroy_shader_module)(d, self.shader_vert, std::ptr::null());
        (fns.destroy_shader_module)(d, self.shader_frag, std::ptr::null());
        (fns.unmap_memory)(d, self.vertex_memory);
        (fns.destroy_buffer)(d, self.vertex_buffer, std::ptr::null());
        (fns.free_memory)(d, self.vertex_memory, std::ptr::null());
        (fns.unmap_memory)(d, self.index_memory);
        (fns.destroy_buffer)(d, self.index_buffer, std::ptr::null());
        (fns.free_memory)(d, self.index_memory, std::ptr::null());
        atlas::destroy_staging(fns, d);
    }
}

fn build_fallback_ui() -> Box<dyn Widget> {
    Box::new(Panel::new(
        Padding::all(10.0),
        [30, 30, 30, 200],
        8.0,
        Box::new(Column::new(
            5.0,
            vec![
                Box::new(Label::new("Ira Overlay", 20.0, [255, 255, 255, 255])),
                Box::new(Row::new(
                    5.0,
                    vec![
                        Box::new(Button::new("Screenshot", 14.0, || {
                            crate::ui::capture::request_screenshot();
                        })),
                        Box::new(Button::new("Record", 14.0, || {
                            crate::ui::capture::toggle_recording();
                        })),
                    ],
                )),
                Box::new(Label::new(
                    "Shift+Tab / Guide to toggle",
                    12.0,
                    [180, 180, 180, 255],
                )),
                Box::new(Label::new(
                    "F12 screenshot | F11 record",
                    12.0,
                    [180, 180, 180, 255],
                )),
            ],
        )),
    ))
}

fn hit_test(bounds: &[Rect], x: f32, y: f32) -> Option<usize> {
    bounds
        .iter()
        .position(|r| x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height)
}
