//! Canvas GPU resources — texture, sampler, descriptors, pipeline.
//!
//! Sibling of `canvas.rs`, which owns per-swapchain state and the
//! upload/draw orchestration. Everything here is construction/destruction;
//! all fields are `pub(crate)` for the orchestrator.

use ash::vk;

use ira_overlay::types::DeviceFns;
use ira_overlay_ipc::{CANVAS_MAX_BYTES, CANVAS_MAX_H, CANVAS_MAX_W, CURSOR_MAX_BYTES};

const CANVAS_VERT_SPV: &[u8] = include_bytes!("../shaders/canvas_vert.spv");
const CANVAS_FRAG_SPV: &[u8] = include_bytes!("../shaders/canvas_frag.spv");

pub(crate) struct CanvasTex {
    pub(crate) image: vk::Image,
    pub(crate) memory: vk::DeviceMemory,
    pub(crate) view: vk::ImageView,
    pub(crate) sampler: vk::Sampler,
    pub(crate) set_layout: vk::DescriptorSetLayout,
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

unsafe impl Send for CanvasTex {}

/// Borrowed upload target: the image + staging pair both texture kinds
/// share, so one copy routine serves panel and cursor.
pub(crate) struct UploadImage<'a> {
    pub image: vk::Image,
    pub staging: vk::Buffer,
    pub staging_ptr: *mut u8,
    pub initialized: &'a mut bool,
}

impl CanvasTex {
    pub(crate) fn upload_image(&mut self) -> UploadImage<'_> {
        UploadImage {
            image: self.image,
            staging: self.staging,
            staging_ptr: self.staging_ptr,
            initialized: &mut self.initialized,
        }
    }
}

/// Cursor texture bundle. Smaller sibling of `CanvasTex`: image + view +
/// descriptor + staging only (vertices reuse the panel VBO per draw).
pub(crate) struct CursorTex {
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
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) xhot: i32,
    pub(crate) yhot: i32,
}

unsafe impl Send for CursorTex {}

impl CursorTex {
    pub(crate) fn upload_image(&mut self) -> UploadImage<'_> {
        UploadImage {
            image: self.image,
            staging: self.staging,
            staging_ptr: self.staging_ptr,
            initialized: &mut self.initialized,
        }
    }
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
    let (image, memory) = create_image(
        fns,
        device,
        physical_device,
        vk::Format::R8G8B8A8_UNORM,
        ira_overlay_ipc::CURSOR_MAX as u32,
        ira_overlay_ipc::CURSOR_MAX as u32,
        vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
    )?;
    let view = create_view(fns, device, image, false)?;
    let (pool, set) = create_set(fns, device, set_layout)?;
    write_descriptor(fns, device, set, view, sampler);
    let (staging, staging_memory, staging_ptr) = create_host_buffer(
        fns,
        device,
        physical_device,
        CURSOR_MAX_BYTES as u64,
        vk::BufferUsageFlags::TRANSFER_SRC,
    )?;
    // Dedicated vertex/index buffers: the panel and cursor draws share one
    // command buffer, so sharing one VBO would let the second upload
    // clobber the first draw's vertices before the GPU executes them.
    let (vbo, vbo_memory, vbo_ptr) = create_host_buffer(
        fns,
        device,
        physical_device,
        4 * 20,
        vk::BufferUsageFlags::VERTEX_BUFFER,
    )?;
    let (ibo, ibo_memory, ibo_ptr) = create_host_buffer(
        fns,
        device,
        physical_device,
        6 * 4,
        vk::BufferUsageFlags::INDEX_BUFFER,
    )?;
    let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
    std::ptr::copy_nonoverlapping(
        indices.as_ptr() as *const u8,
        ibo_ptr as *mut u8,
        std::mem::size_of_val(&indices),
    );
    Some(CursorTex {
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
        vbo_ptr: vbo_ptr as *mut u8,
        ibo,
        ibo_memory,
        initialized: false,
        last_seq: 0,
        w: 0,
        h: 0,
        xhot: 0,
        yhot: 0,
    })
}

/// Allocates one descriptor set from a fresh single-set pool.
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
    let d = device;
    (fns.destroy_image_view)(d, tex.view, std::ptr::null());
    (fns.destroy_image)(d, tex.image, std::ptr::null());
    (fns.free_memory)(d, tex.memory, std::ptr::null());
    (fns.destroy_descriptor_pool)(d, tex.pool, std::ptr::null());
    (fns.unmap_memory)(d, tex.staging_memory);
    (fns.destroy_buffer)(d, tex.staging, std::ptr::null());
    (fns.free_memory)(d, tex.staging_memory, std::ptr::null());
    (fns.unmap_memory)(d, tex.vbo_memory);
    (fns.destroy_buffer)(d, tex.vbo, std::ptr::null());
    (fns.free_memory)(d, tex.vbo_memory, std::ptr::null());
    (fns.unmap_memory)(d, tex.ibo_memory);
    (fns.destroy_buffer)(d, tex.ibo, std::ptr::null());
    (fns.free_memory)(d, tex.ibo_memory, std::ptr::null());
}

/// Creates the canvas texture + sampler + descriptor + staging resources.
/// Returns `None` on any failure — the game presents without overlay.
pub(crate) unsafe fn create(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
) -> Option<CanvasTex> {
    let (image, memory) = create_canvas_image(fns, device, physical_device)?;
    let view = create_view(fns, device, image, true)?;
    let sampler = create_sampler(fns, device);
    let (set_layout, pool, set) = create_descriptors(fns, device)?;
    write_descriptor(fns, device, set, view, sampler);

    let (staging, staging_memory, staging_ptr) = create_host_buffer(
        fns,
        device,
        physical_device,
        CANVAS_MAX_BYTES as u64,
        vk::BufferUsageFlags::TRANSFER_SRC,
    )?;
    let (vbo, vbo_memory, vbo_ptr) = create_host_buffer(
        fns,
        device,
        physical_device,
        4 * 20,
        vk::BufferUsageFlags::VERTEX_BUFFER,
    )?;
    let (ibo, ibo_memory, ibo_ptr) = create_host_buffer(
        fns,
        device,
        physical_device,
        6 * 4,
        vk::BufferUsageFlags::INDEX_BUFFER,
    )?;
    let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
    std::ptr::copy_nonoverlapping(
        indices.as_ptr() as *const u8,
        ibo_ptr as *mut u8,
        std::mem::size_of_val(&indices),
    );

    Some(CanvasTex {
        image,
        memory,
        view,
        sampler,
        set_layout,
        pool,
        set,
        staging,
        staging_memory,
        staging_ptr: staging_ptr as *mut u8,
        vbo,
        vbo_memory,
        vbo_ptr: vbo_ptr as *mut u8,
        ibo,
        ibo_memory,
        initialized: false,
        last_seq: 0,
    })
}

/// Creates the textured-quad pipeline. Returns
/// (layout, pipeline, vert_module, frag_module) for swapchain bookkeeping.
pub(crate) unsafe fn create_pipeline(
    fns: DeviceFns,
    device: vk::Device,
    render_pass: vk::RenderPass,
    set_layout: vk::DescriptorSetLayout,
) -> (
    vk::PipelineLayout,
    vk::Pipeline,
    vk::ShaderModule,
    vk::ShaderModule,
) {
    let vert_code = words(CANVAS_VERT_SPV);
    let vert_info = vk::ShaderModuleCreateInfo::default().code(&vert_code);
    let mut shader_vert = vk::ShaderModule::null();
    let _ = (fns.create_shader_module)(device, &vert_info, std::ptr::null(), &mut shader_vert);

    let frag_code = words(CANVAS_FRAG_SPV);
    let frag_info = vk::ShaderModuleCreateInfo::default().code(&frag_code);
    let mut shader_frag = vk::ShaderModule::null();
    let _ = (fns.create_shader_module)(device, &frag_info, std::ptr::null(), &mut shader_frag);

    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(shader_vert)
            .name(c"main"),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(shader_frag)
            .name(c"main"),
    ];

    let binding = vk::VertexInputBindingDescription::default()
        .binding(0)
        .stride(20)
        .input_rate(vk::VertexInputRate::VERTEX);
    let attributes = [
        vk::VertexInputAttributeDescription::default()
            .binding(0)
            .location(0)
            .format(vk::Format::R32G32_SFLOAT)
            .offset(0),
        vk::VertexInputAttributeDescription::default()
            .binding(0)
            .location(1)
            .format(vk::Format::R32G32_SFLOAT)
            .offset(8),
        vk::VertexInputAttributeDescription::default()
            .binding(0)
            .location(2)
            .format(vk::Format::R8G8B8A8_UNORM)
            .offset(16),
    ];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(std::slice::from_ref(&binding))
        .vertex_attribute_descriptions(&attributes);

    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let viewport = vk::Viewport::default();
    let scissor = vk::Rect2D::default();
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewports(std::slice::from_ref(&viewport))
        .scissors(std::slice::from_ref(&scissor));
    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .line_width(1.0)
        .cull_mode(vk::CullModeFlags::NONE);
    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    // Canvas pixels are premultiplied — same factors as the glyph pipeline.
    let blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA)
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::ONE)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .alpha_blend_op(vk::BlendOp::ADD);
    let color_blend = vk::PipelineColorBlendStateCreateInfo::default()
        .attachments(std::slice::from_ref(&blend_attachment));
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let push_constant = vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
        .offset(0)
        .size(24);
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&set_layout))
        .push_constant_ranges(std::slice::from_ref(&push_constant));
    let mut pipeline_layout = vk::PipelineLayout::null();
    let _ =
        (fns.create_pipeline_layout)(device, &layout_info, std::ptr::null(), &mut pipeline_layout);

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterizer)
        .multisample_state(&multisampling)
        .color_blend_state(&color_blend)
        .dynamic_state(&dynamic)
        .layout(pipeline_layout)
        .render_pass(render_pass);
    let mut pipeline = vk::Pipeline::null();
    let _ = (fns.create_graphics_pipelines)(
        device,
        vk::PipelineCache::null(),
        1,
        &pipeline_info,
        std::ptr::null(),
        &mut pipeline,
    );

    (pipeline_layout, pipeline, shader_vert, shader_frag)
}

/// Destroys all canvas resources. No GPU work may reference them.
pub(crate) unsafe fn destroy(fns: DeviceFns, device: vk::Device, tex: &CanvasTex) {
    let d = device;
    (fns.destroy_sampler)(d, tex.sampler, std::ptr::null());
    (fns.destroy_image_view)(d, tex.view, std::ptr::null());
    (fns.destroy_image)(d, tex.image, std::ptr::null());
    (fns.free_memory)(d, tex.memory, std::ptr::null());
    (fns.destroy_descriptor_pool)(d, tex.pool, std::ptr::null());
    (fns.destroy_descriptor_set_layout)(d, tex.set_layout, std::ptr::null());
    (fns.unmap_memory)(d, tex.staging_memory);
    (fns.destroy_buffer)(d, tex.staging, std::ptr::null());
    (fns.free_memory)(d, tex.staging_memory, std::ptr::null());
    (fns.unmap_memory)(d, tex.vbo_memory);
    (fns.destroy_buffer)(d, tex.vbo, std::ptr::null());
    (fns.free_memory)(d, tex.vbo_memory, std::ptr::null());
    (fns.unmap_memory)(d, tex.ibo_memory);
    (fns.destroy_buffer)(d, tex.ibo, std::ptr::null());
    (fns.free_memory)(d, tex.ibo_memory, std::ptr::null());
}

fn words(spv: &[u8]) -> Vec<u32> {
    let (chunks, _) = spv.as_chunks::<4>();
    chunks.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

unsafe fn find_memory_type(
    fns: DeviceFns,
    physical_device: vk::PhysicalDevice,
    type_filter: u32,
    properties: vk::MemoryPropertyFlags,
) -> Option<u32> {
    let mut props = vk::PhysicalDeviceMemoryProperties::default();
    (fns.get_mem_props)(physical_device, &mut props);
    for i in 0..props.memory_type_count as usize {
        if (type_filter & (1 << i)) != 0
            && props.memory_types[i].property_flags.contains(properties)
        {
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

/// Device-local canvas texture at max extent; the live frame occupies the
/// top-left `w×h` subregion.
unsafe fn create_canvas_image(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
) -> Option<(vk::Image, vk::DeviceMemory)> {
    create_image(
        fns,
        device,
        physical_device,
        vk::Format::R8G8B8A8_UNORM,
        CANVAS_MAX_W as u32,
        CANVAS_MAX_H as u32,
        vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
    )
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

unsafe fn create_descriptors(
    fns: DeviceFns,
    device: vk::Device,
) -> Option<(
    vk::DescriptorSetLayout,
    vk::DescriptorPool,
    vk::DescriptorSet,
)> {
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
        (fns.destroy_descriptor_set_layout)(device, set_layout, std::ptr::null());
        return None;
    }

    let alloc_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(std::slice::from_ref(&set_layout));
    let mut set = vk::DescriptorSet::null();
    if (fns.allocate_descriptor_sets)(device, &alloc_info, &mut set) != vk::Result::SUCCESS {
        (fns.destroy_descriptor_pool)(device, pool, std::ptr::null());
        (fns.destroy_descriptor_set_layout)(device, set_layout, std::ptr::null());
        return None;
    }
    Some((set_layout, pool, set))
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
