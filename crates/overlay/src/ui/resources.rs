use ash::vk;

use crate::types::{DeviceFns, UI_FRAG_SPV, UI_VERT_SPV};

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

pub(crate) unsafe fn create_buffer(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    size: vk::DeviceSize,
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

pub(crate) unsafe fn create_image(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    format: vk::Format,
    extent: vk::Extent2D,
    usage: vk::ImageUsageFlags,
) -> Option<(vk::Image, vk::DeviceMemory)> {
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
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

pub(crate) unsafe fn create_atlas_texture(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    cmd_pool: vk::CommandPool,
) -> Option<(vk::Image, vk::DeviceMemory, vk::ImageView)> {
    let extent = vk::Extent2D {
        width: 2048,
        height: 2048,
    };
    let (image, image_memory) = create_image(
        fns,
        device,
        physical_device,
        vk::Format::R8G8B8A8_UNORM,
        extent,
        vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
    )?;

    let mut cmd = vk::CommandBuffer::null();
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(cmd_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let _ = (fns.allocate_cmd_buffers)(device, &alloc_info, &mut cmd);

    let begin_info =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    let _ = (fns.begin_cmd_buffer)(cmd, &begin_info);

    let subresource = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };

    let barrier = vk::ImageMemoryBarrier::default()
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(subresource);
    (fns.cmd_pipeline_barrier)(
        cmd,
        vk::PipelineStageFlags::TOP_OF_PIPE,
        vk::PipelineStageFlags::FRAGMENT_SHADER,
        vk::DependencyFlags::empty(),
        0,
        std::ptr::null(),
        0,
        std::ptr::null(),
        1,
        &barrier,
    );

    let _ = (fns.end_cmd_buffer)(cmd);

    let mut queue = vk::Queue::null();
    (fns.get_device_queue)(device, 0, 0, &mut queue);
    let mut fence = vk::Fence::null();
    let fence_info = vk::FenceCreateInfo::default();
    let _ = (fns.create_fence)(device, &fence_info, std::ptr::null(), &mut fence);
    let submit_info = vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&cmd));
    let _ = (fns.queue_submit)(queue, 1, &submit_info, fence);
    let _ = (fns.wait_for_fences)(device, 1, &fence, vk::TRUE, 2_000_000_000);
    (fns.destroy_fence)(device, fence, std::ptr::null());
    (fns.free_cmd_buffers)(device, cmd_pool, 1, &cmd);

    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });
    let mut view = vk::ImageView::null();
    let _ = (fns.create_image_view)(device, &view_info, std::ptr::null(), &mut view);

    Some((image, image_memory, view))
}

pub(crate) unsafe fn create_sampler(fns: DeviceFns, device: vk::Device) -> vk::Sampler {
    let info = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::LINEAR)
        .min_filter(vk::Filter::LINEAR)
        .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .border_color(vk::BorderColor::INT_TRANSPARENT_BLACK)
        .unnormalized_coordinates(false);
    let mut sampler = vk::Sampler::null();
    let _ = (fns.create_sampler)(device, &info, std::ptr::null(), &mut sampler);
    sampler
}

pub(crate) unsafe fn create_descriptors(
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
    let vert_code: Vec<u32> = {
        let (chunks, _) = UI_VERT_SPV.as_chunks::<4>();
        chunks.iter().map(|c| u32::from_le_bytes(*c)).collect()
    };
    let vert_info = vk::ShaderModuleCreateInfo::default().code(&vert_code);
    let mut shader_vert = vk::ShaderModule::null();
    let _ = (fns.create_shader_module)(device, &vert_info, std::ptr::null(), &mut shader_vert);

    let frag_code: Vec<u32> = {
        let (chunks, _) = UI_FRAG_SPV.as_chunks::<4>();
        chunks.iter().map(|c| u32::from_le_bytes(*c)).collect()
    };
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

pub(crate) unsafe fn create_vertex_buffer(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    size: vk::DeviceSize,
) -> Option<(vk::Buffer, vk::DeviceMemory, *mut std::ffi::c_void)> {
    create_buffer(
        fns,
        device,
        physical_device,
        size,
        vk::BufferUsageFlags::VERTEX_BUFFER,
    )
}

pub(crate) unsafe fn create_index_buffer(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    size: vk::DeviceSize,
) -> Option<(vk::Buffer, vk::DeviceMemory, *mut std::ffi::c_void)> {
    create_buffer(
        fns,
        device,
        physical_device,
        size,
        vk::BufferUsageFlags::INDEX_BUFFER,
    )
}
