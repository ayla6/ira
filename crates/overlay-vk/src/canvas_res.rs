//! Canvas GPU resources — texture, sampler, descriptors, pipeline.
//!
//! Sibling of `canvas.rs`, which owns per-swapchain state and the
//! upload/draw orchestration. Everything here is construction/destruction;
//! all fields are `pub(crate)` for the orchestrator.

use ash::vk;

use ira_overlay::types::DeviceFns;
use ira_overlay_ipc::{CANVAS_MAX_BYTES, CANVAS_MAX_H, CANVAS_MAX_W, CURSOR_MAX_BYTES};

pub(crate) struct CanvasTex {
    pub(crate) core: TexCore,
    pub(crate) sampler: vk::Sampler,
    pub(crate) set_layout: vk::DescriptorSetLayout,
}

unsafe impl Send for CanvasTex {}

/// Texture resources shared by the panel and cursor bundles: image + view +
/// descriptor + staging + quad buffers + upload state. The panel adds its
/// sampler/layout on top; the cursor adds its size/hotspot.
pub(crate) struct TexCore {
    pub(crate) image: vk::Image,
    pub(crate) memory: vk::DeviceMemory,
    pub(crate) view: vk::ImageView,
    pub(crate) pool: vk::DescriptorPool,
    pub(crate) set: vk::DescriptorSet,
    pub(crate) staging: vk::Buffer,
    pub(crate) staging_memory: vk::DeviceMemory,
    pub(crate) staging_ptr: *mut u8,
    pub(crate) vbo: vk::Buffer,
    pub(crate) vbo_memory: vk::DeviceMemory,
    pub(crate) vbo_ptr: *mut u8,
    pub(crate) ibo: vk::Buffer,
    pub(crate) ibo_memory: vk::DeviceMemory,
    pub(crate) initialized: bool,
    pub(crate) last_seq: u32,
}

unsafe impl Send for TexCore {}

/// Borrowed upload target: the image + staging pair both texture kinds
/// share, so one copy routine serves panel and cursor.
pub(crate) struct UploadImage<'a> {
    pub image: vk::Image,
    pub staging: vk::Buffer,
    pub staging_ptr: *mut u8,
    pub initialized: &'a mut bool,
}

impl TexCore {
    pub(crate) fn upload_image(&mut self) -> UploadImage<'_> {
        UploadImage {
            image: self.image,
            staging: self.staging,
            staging_ptr: self.staging_ptr,
            initialized: &mut self.initialized,
        }
    }
}

/// Cursor texture bundle: shared core plus size/hotspot (vertices reuse
/// per-draw uploads, so no VBO sharing hazard beyond the dedicated pair
/// in the core).
pub(crate) struct CursorTex {
    pub(crate) core: TexCore,
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) xhot: i32,
    pub(crate) yhot: i32,
}

unsafe impl Send for CursorTex {}

/// Allocates the vertex/index buffer pair for one textured quad plus the
/// fixed triangle indices. Shared by both bundles.
unsafe fn create_quad_buffers(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
) -> Option<(
    vk::Buffer,
    vk::DeviceMemory,
    *mut u8,
    vk::Buffer,
    vk::DeviceMemory,
)> {
    let (vbo, vbo_memory, vbo_ptr) = create_host_buffer(
        fns,
        device,
        physical_device,
        super::canvas::QUAD_VBO_BYTES,
        vk::BufferUsageFlags::VERTEX_BUFFER,
    )?;
    let (ibo, ibo_memory, ibo_ptr) = create_host_buffer(
        fns,
        device,
        physical_device,
        super::canvas::QUAD_IBO_BYTES,
        vk::BufferUsageFlags::INDEX_BUFFER,
    )?;
    let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
    std::ptr::copy_nonoverlapping(
        indices.as_ptr() as *const u8,
        ibo_ptr as *mut u8,
        std::mem::size_of_val(&indices),
    );
    Some((vbo, vbo_memory, vbo_ptr as *mut u8, ibo, ibo_memory))
}

/// Parameters for one texture bundle: dimensions, staging size, swizzle,
/// and the sampler/layout it is described with.
struct TexParams {
    width: u32,
    height: u32,
    staging_bytes: u64,
    swizzle_rb: bool,
    sampler: vk::Sampler,
    set_layout: vk::DescriptorSetLayout,
}

/// Allocates image + view + descriptor + staging + quad buffers: everything
/// both bundles share. Callers pass their sampler/layout (fresh for the
/// panel, shared for the cursor) and get back the core.
unsafe fn create_core(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    params: TexParams,
) -> Option<TexCore> {
    let (image, memory) = create_image(
        fns,
        device,
        physical_device,
        vk::Format::R8G8B8A8_UNORM,
        params.width,
        params.height,
        vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
    )?;
    let view = create_view(fns, device, image, params.swizzle_rb)?;
    let (pool, set) = create_set(fns, device, params.set_layout)?;
    write_descriptor(fns, device, set, view, params.sampler);
    let (staging, staging_memory, staging_ptr) = create_host_buffer(
        fns,
        device,
        physical_device,
        params.staging_bytes,
        vk::BufferUsageFlags::TRANSFER_SRC,
    )?;
    let (vbo, vbo_memory, vbo_ptr, ibo, ibo_memory) =
        create_quad_buffers(fns, device, physical_device)?;
    Some(TexCore {
        image,
        memory,
        view,
        pool,
        set,
        staging,
        staging_memory,
        staging_ptr: staging_ptr as *mut u8,
        vbo,
        vbo_memory,
        vbo_ptr,
        ibo,
        ibo_memory,
        initialized: false,
        last_seq: 0,
    })
}

/// Destroys the shared core. No GPU work may reference it.
unsafe fn destroy_core(fns: DeviceFns, device: vk::Device, core: &TexCore) {
    let d = device;
    (fns.destroy_image_view)(d, core.view, std::ptr::null());
    (fns.destroy_image)(d, core.image, std::ptr::null());
    (fns.free_memory)(d, core.memory, std::ptr::null());
    (fns.destroy_descriptor_pool)(d, core.pool, std::ptr::null());
    (fns.unmap_memory)(d, core.staging_memory);
    (fns.destroy_buffer)(d, core.staging, std::ptr::null());
    (fns.free_memory)(d, core.staging_memory, std::ptr::null());
    (fns.unmap_memory)(d, core.vbo_memory);
    (fns.destroy_buffer)(d, core.vbo, std::ptr::null());
    (fns.free_memory)(d, core.vbo_memory, std::ptr::null());
    (fns.unmap_memory)(d, core.ibo_memory);
    (fns.destroy_buffer)(d, core.ibo, std::ptr::null());
    (fns.free_memory)(d, core.ibo_memory, std::ptr::null());
}

/// Creates the cursor texture (128×128 RGBA, no swizzle) + descriptor +
/// staging. Shares the panel sampler.
pub(crate) unsafe fn create_cursor(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    sampler: vk::Sampler,
    set_layout: vk::DescriptorSetLayout,
) -> Option<CursorTex> {
    let core = create_core(
        fns,
        device,
        physical_device,
        TexParams {
            width: ira_overlay_ipc::CURSOR_MAX as u32,
            height: ira_overlay_ipc::CURSOR_MAX as u32,
            staging_bytes: CURSOR_MAX_BYTES as u64,
            swizzle_rb: false,
            sampler,
            set_layout,
        },
    )?;
    Some(CursorTex {
        core,
        w: 0,
        h: 0,
        xhot: 0,
        yhot: 0,
    })
}

unsafe fn create_set(
    fns: DeviceFns,
    device: vk::Device,
    set_layout: vk::DescriptorSetLayout,
) -> Option<(vk::DescriptorPool, vk::DescriptorSet)> {
    let pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .descriptor_count(1);
    let pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(std::slice::from_ref(&pool_size));
    let mut pool = vk::DescriptorPool::null();
    if (fns.create_descriptor_pool)(device, &pool_info, std::ptr::null(), &mut pool)
        != vk::Result::SUCCESS
    {
        return None;
    }
    let alloc_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(std::slice::from_ref(&set_layout));
    let mut set = vk::DescriptorSet::null();
    if (fns.allocate_descriptor_sets)(device, &alloc_info, &mut set) != vk::Result::SUCCESS {
        (fns.destroy_descriptor_pool)(device, pool, std::ptr::null());
        return None;
    }
    Some((pool, set))
}

/// Destroys cursor resources. No GPU work may reference them.
pub(crate) unsafe fn destroy_cursor(fns: DeviceFns, device: vk::Device, tex: &CursorTex) {
    destroy_core(fns, device, &tex.core);
}

/// Creates the canvas texture + sampler + descriptor + staging resources.
/// Returns `None` on any failure — the game presents without overlay.
pub(crate) unsafe fn create(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
) -> Option<CanvasTex> {
    let sampler = create_sampler(fns, device);
    let set_layout = create_layout(fns, device)?;
    let core = create_core(
        fns,
        device,
        physical_device,
        TexParams {
            width: CANVAS_MAX_W as u32,
            height: CANVAS_MAX_H as u32,
            staging_bytes: CANVAS_MAX_BYTES as u64,
            swizzle_rb: true,
            sampler,
            set_layout,
        },
    )?;
    Some(CanvasTex {
        core,
        sampler,
        set_layout,
    })
}

/// Destroys all canvas resources. No GPU work may reference them.
pub(crate) unsafe fn destroy(fns: DeviceFns, device: vk::Device, tex: &CanvasTex) {
    let d = device;
    destroy_core(fns, device, &tex.core);
    (fns.destroy_sampler)(d, tex.sampler, std::ptr::null());
    (fns.destroy_descriptor_set_layout)(d, tex.set_layout, std::ptr::null());
}

unsafe fn find_memory_type(
    fns: DeviceFns,
    physical_device: vk::PhysicalDevice,
    type_filter: u32,
    properties: vk::MemoryPropertyFlags,
) -> Option<u32> {
    let mut props = vk::PhysicalDeviceMemoryProperties::default();
    (fns.get_mem_props)(physical_device, &mut props);
    for (i, mem_type) in props
        .memory_types
        .iter()
        .take(props.memory_type_count as usize)
        .enumerate()
    {
        if (type_filter & (1 << i)) != 0 && mem_type.property_flags.contains(properties) {
            return Some(i as u32);
        }
    }
    None
}

unsafe fn create_host_buffer(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    size: u64,
    usage: vk::BufferUsageFlags,
) -> Option<(vk::Buffer, vk::DeviceMemory, *mut std::ffi::c_void)> {
    let info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let mut buffer = vk::Buffer::null();
    if (fns.create_buffer)(device, &info, std::ptr::null(), &mut buffer) != vk::Result::SUCCESS {
        return None;
    }
    let mut reqs = vk::MemoryRequirements::default();
    (fns.get_buffer_memory_requirements)(device, buffer, &mut reqs);
    let mem_type = find_memory_type(
        fns,
        physical_device,
        reqs.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;
    let alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(reqs.size)
        .memory_type_index(mem_type);
    let mut memory = vk::DeviceMemory::null();
    if (fns.allocate_memory)(device, &alloc, std::ptr::null(), &mut memory) != vk::Result::SUCCESS {
        (fns.destroy_buffer)(device, buffer, std::ptr::null());
        return None;
    }
    let _ = (fns.bind_buffer_memory)(device, buffer, memory, 0);
    let mut ptr = std::ptr::null_mut();
    if (fns.map_memory)(
        device,
        memory,
        0,
        reqs.size,
        vk::MemoryMapFlags::empty(),
        &mut ptr,
    ) != vk::Result::SUCCESS
    {
        (fns.free_memory)(device, memory, std::ptr::null());
        (fns.destroy_buffer)(device, buffer, std::ptr::null());
        return None;
    }
    Some((buffer, memory, ptr))
}

unsafe fn create_image(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    format: vk::Format,
    width: u32,
    height: u32,
    usage: vk::ImageUsageFlags,
) -> Option<(vk::Image, vk::DeviceMemory)> {
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    let mut image = vk::Image::null();
    if (fns.create_image)(device, &info, std::ptr::null(), &mut image) != vk::Result::SUCCESS {
        return None;
    }
    let mut reqs = vk::MemoryRequirements::default();
    (fns.get_image_memory_requirements)(device, image, &mut reqs);
    let mem_type = find_memory_type(
        fns,
        physical_device,
        reqs.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;
    let alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(reqs.size)
        .memory_type_index(mem_type);
    let mut memory = vk::DeviceMemory::null();
    if (fns.allocate_memory)(device, &alloc, std::ptr::null(), &mut memory) != vk::Result::SUCCESS {
        (fns.destroy_image)(device, image, std::ptr::null());
        return None;
    }
    let _ = (fns.bind_image_memory)(device, image, memory, 0);
    Some((image, memory))
}

/// Descriptor-set layout for one combined image sampler. The pool + set
/// allocation half lives in `create_set`, shared with the cursor path via
/// `create_core`.
unsafe fn create_layout(fns: DeviceFns, device: vk::Device) -> Option<vk::DescriptorSetLayout> {
    let binding = vk::DescriptorSetLayoutBinding::default()
        .binding(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::FRAGMENT);
    let layout_info =
        vk::DescriptorSetLayoutCreateInfo::default().bindings(std::slice::from_ref(&binding));
    let mut set_layout = vk::DescriptorSetLayout::null();
    if (fns.create_descriptor_set_layout)(device, &layout_info, std::ptr::null(), &mut set_layout)
        != vk::Result::SUCCESS
    {
        return None;
    }
    Some(set_layout)
}

/// View with optional R↔B swizzle. Panel bytes are Cairo ARGB32 (BGRA on
/// little-endian), sampled here as RGBA; cursor bytes arrive as RGBA and
/// need no swizzle.
unsafe fn create_view(
    fns: DeviceFns,
    device: vk::Device,
    image: vk::Image,
    swizzle_rb: bool,
) -> Option<vk::ImageView> {
    let (r, b) = if swizzle_rb {
        (vk::ComponentSwizzle::B, vk::ComponentSwizzle::R)
    } else {
        (vk::ComponentSwizzle::R, vk::ComponentSwizzle::B)
    };
    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .components(vk::ComponentMapping {
            r,
            g: vk::ComponentSwizzle::G,
            b,
            a: vk::ComponentSwizzle::A,
        })
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });
    let mut view = vk::ImageView::null();
    if (fns.create_image_view)(device, &view_info, std::ptr::null(), &mut view)
        != vk::Result::SUCCESS
    {
        return None;
    }
    Some(view)
}

unsafe fn create_sampler(fns: DeviceFns, device: vk::Device) -> vk::Sampler {
    let info = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::LINEAR)
        .min_filter(vk::Filter::LINEAR)
        .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .unnormalized_coordinates(false);
    let mut sampler = vk::Sampler::null();
    let _ = (fns.create_sampler)(device, &info, std::ptr::null(), &mut sampler);
    sampler
}

unsafe fn write_descriptor(
    fns: DeviceFns,
    device: vk::Device,
    set: vk::DescriptorSet,
    view: vk::ImageView,
    sampler: vk::Sampler,
) {
    let image_info = vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .image_view(view)
        .sampler(sampler);
    let write = vk::WriteDescriptorSet::default()
        .dst_set(set)
        .dst_binding(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .image_info(std::slice::from_ref(&image_info));
    (fns.update_descriptor_sets)(device, 1, &write, 0, std::ptr::null());
}
