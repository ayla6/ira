use std::collections::HashMap;

use ash::vk;
use ash::vk::Handle;

use crate::types::*;
use ira_overlay::types::DeviceFns;

pub(crate) unsafe extern "system" fn create_swapchain(
    device: vk::Device,
    create_info: *const vk::SwapchainCreateInfoKHR,
    allocator: *const vk::AllocationCallbacks,
    swapchain: *mut vk::SwapchainKHR,
) -> vk::Result {
    let (fns, physical_device) = {
        let map = DEVICES.lock().unwrap();
        let Some(dd) = map
            .as_ref()
            .and_then(|m| m.get(&(device.as_raw() as usize)))
        else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        let Some(fns) = dd.fns else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        (fns, dd.physical_device)
    };

    let ci = *create_info;
    // Don't force TRANSFER_SRC — some emulators (shadPS4) crash when the
    // swapchain has unexpected usage flags. Screenshot capture will use
    // a blit to a separate image instead.
    crate::present::DEVICE_LOST.store(false, std::sync::atomic::Ordering::Relaxed);
    let result = (fns.create_swapchain)(device, &ci, allocator, swapchain);
    if result != vk::Result::SUCCESS {
        eprintln!("ira-overlay: swapchain creation failed: {:?}", result);
        return result;
    }

    eprintln!(
        "ira-overlay: swapchain created {}x{}",
        ci.image_extent.width, ci.image_extent.height
    );

    // Reset the present counter so the overlay "ready" delay restarts after
    // swapchain recreation. Games like shadPS4 create multiple swapchains
    // during loading (1280x720 → 1920x1080); the overlay should only become
    // toggleable after the final swapchain is stable.
    crate::shim_bridge::reset_present_count();

    let create_info = &ci;
    let sc = *swapchain;

    let mut image_count = 0u32;
    let _ = (fns.get_swapchain_images)(device, sc, &mut image_count, std::ptr::null_mut());
    let mut images = vec![vk::Image::null(); image_count as usize];
    let _ = (fns.get_swapchain_images)(device, sc, &mut image_count, images.as_mut_ptr());

    let format = create_info.image_format;
    let extent = create_info.image_extent;

    let render_pass = create_render_pass(fns, device, format);
    let (pipeline, pipeline_layout, shader_vert, shader_frag) =
        create_pipeline(fns, device, render_pass, extent);
    let (framebuffers, image_views) =
        create_framebuffers(fns, device, render_pass, &images, extent, format);

    let cmd_pool = {
        let pool_info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let mut pool = vk::CommandPool::null();
        let _ = (fns.create_command_pool)(device, &pool_info, std::ptr::null(), &mut pool);
        pool
    };

    let mut cmd_buffers = vec![vk::CommandBuffer::null(); image_count as usize];
    {
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(cmd_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(image_count);
        let _ = (fns.allocate_cmd_buffers)(device, &alloc_info, cmd_buffers.as_mut_ptr());
    }

    let mut semaphores = Vec::with_capacity(image_count as usize);
    let mut fences = Vec::with_capacity(image_count as usize);
    let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
    for _ in 0..image_count {
        let mut sem = vk::Semaphore::null();
        let info = vk::SemaphoreCreateInfo::default();
        let _ = (fns.create_semaphore)(device, &info, std::ptr::null(), &mut sem);
        semaphores.push(sem);
        let mut fence = vk::Fence::null();
        let _ = (fns.create_fence)(device, &fence_info, std::ptr::null(), &mut fence);
        fences.push(fence);
    }

    let ui_enabled = std::env::var_os("IRA_OVERLAY_DISABLE_UI").is_none();
    let ui_renderer = if ui_enabled {
        ira_overlay::ui::UiRenderer::new(fns, device, physical_device, cmd_pool, render_pass)
    } else {
        None
    };

    ira_overlay::ui::capture::init(fns, device, physical_device, extent, format);

    SWAPCHAINS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(
            sc.as_raw(),
            SwapchainData {
                device,
                fns,
                images,
                extent,
                render_pass,
                pipeline,
                pipeline_layout,
                shader_vert,
                shader_frag,
                framebuffers,
                image_views,
                cmd_pool,
                cmd_buffers,
                semaphores,
                fences,
                ui_enabled,
                ui_renderer,
            },
        );

    result
}

pub(crate) unsafe extern "system" fn destroy_swapchain(
    device: vk::Device,
    swapchain: vk::SwapchainKHR,
    allocator: *const vk::AllocationCallbacks,
) {
    let sc_data = SWAPCHAINS
        .lock()
        .unwrap()
        .as_mut()
        .and_then(|m| m.remove(&(swapchain.as_raw())));
    if let Some(sc) = sc_data {
        let fns = sc.fns;

        let _ = (fns.device_wait_idle)(sc.device);

        ira_overlay::ui::capture::drain_pending();

        if let Some(ui) = sc.ui_renderer {
            ui.destroy(fns);
        }
        (fns.destroy_swapchain)(device, swapchain, allocator);
        for fb in &sc.framebuffers {
            (fns.destroy_framebuffer)(device, *fb, std::ptr::null());
        }
        for iv in &sc.image_views {
            (fns.destroy_image_view)(device, *iv, std::ptr::null());
        }
        for sem in &sc.semaphores {
            (fns.destroy_semaphore)(device, *sem, std::ptr::null());
        }
        for fence in &sc.fences {
            (fns.destroy_fence)(device, *fence, std::ptr::null());
        }
        (fns.destroy_pipeline)(device, sc.pipeline, std::ptr::null());
        (fns.destroy_pipeline_layout)(device, sc.pipeline_layout, std::ptr::null());
        (fns.destroy_shader_module)(device, sc.shader_vert, std::ptr::null());
        (fns.destroy_shader_module)(device, sc.shader_frag, std::ptr::null());
        (fns.destroy_render_pass)(device, sc.render_pass, std::ptr::null());
        (fns.destroy_command_pool)(device, sc.cmd_pool, std::ptr::null());
    } else {
        let fns = {
            let map = DEVICES.lock().unwrap();
            map.as_ref()
                .and_then(|m| m.get(&(device.as_raw() as usize)))
                .and_then(|d| d.fns)
        };
        if let Some(fns) = fns {
            (fns.destroy_swapchain)(device, swapchain, allocator);
        }
    }
}

unsafe fn create_render_pass(
    fns: DeviceFns,
    device: vk::Device,
    format: vk::Format,
) -> vk::RenderPass {
    let attachment = vk::AttachmentDescription::default()
        .format(format)
        .samples(vk::SampleCountFlags::TYPE_1)
        // The injected layer renders into the game's existing swapchain image.
        // Clearing here would replace the game with transparent black before
        // the overlay is drawn.
        .load_op(vk::AttachmentLoadOp::LOAD)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

    let color_ref = vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

    let subpass = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(std::slice::from_ref(&color_ref));

    let dependency = vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::TOP_OF_PIPE)
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .src_access_mask(vk::AccessFlags::NONE)
        .dst_access_mask(
            vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        );

    let rp_info = vk::RenderPassCreateInfo::default()
        .attachments(std::slice::from_ref(&attachment))
        .subpasses(std::slice::from_ref(&subpass))
        .dependencies(std::slice::from_ref(&dependency));

    let mut rp = vk::RenderPass::null();
    let _ = (fns.create_render_pass)(device, &rp_info, std::ptr::null(), &mut rp);
    rp
}

unsafe fn create_pipeline(
    fns: DeviceFns,
    device: vk::Device,
    render_pass: vk::RenderPass,
    extent: vk::Extent2D,
) -> (
    vk::Pipeline,
    vk::PipelineLayout,
    vk::ShaderModule,
    vk::ShaderModule,
) {
    let vert_code: Vec<u32> = {
        let (chunks, _) = VERT_SPV.as_chunks::<4>();
        chunks.iter().map(|c| u32::from_le_bytes(*c)).collect()
    };
    let vert_info = vk::ShaderModuleCreateInfo::default().code(&vert_code);
    let mut shader_vert = vk::ShaderModule::null();
    let _ = (fns.create_shader_module)(device, &vert_info, std::ptr::null(), &mut shader_vert);

    let frag_code: Vec<u32> = {
        let (chunks, _) = FRAG_SPV.as_chunks::<4>();
        chunks.iter().map(|c| u32::from_le_bytes(*c)).collect()
    };
    let frag_info = vk::ShaderModuleCreateInfo::default().code(&frag_code);
    let mut shader_frag = vk::ShaderModule::null();
    let _ = (fns.create_shader_module)(device, &frag_info, std::ptr::null(), &mut shader_frag);

    let vert_stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::VERTEX)
        .module(shader_vert)
        .name(c"main");
    let frag_stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::FRAGMENT)
        .module(shader_frag)
        .name(c"main");

    let stages = [vert_stage, frag_stage];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);

    let viewport = vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: extent.width as f32,
        height: extent.height as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };
    let scissor = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent,
    };
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewports(std::slice::from_ref(&viewport))
        .scissors(std::slice::from_ref(&scissor));

    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .depth_clamp_enable(false)
        .rasterizer_discard_enable(false)
        .polygon_mode(vk::PolygonMode::FILL)
        .line_width(1.0)
        .cull_mode(vk::CullModeFlags::NONE);

    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .sample_shading_enable(false)
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);

    let blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA)
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
        .alpha_blend_op(vk::BlendOp::ADD);

    let color_blend = vk::PipelineColorBlendStateCreateInfo::default()
        .attachments(std::slice::from_ref(&blend_attachment));

    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let layout_info = vk::PipelineLayoutCreateInfo::default();
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
        &pipeline_info as *const vk::GraphicsPipelineCreateInfo,
        std::ptr::null(),
        &mut pipeline as *mut vk::Pipeline,
    );

    (pipeline, pipeline_layout, shader_vert, shader_frag)
}

unsafe fn create_framebuffers(
    fns: DeviceFns,
    device: vk::Device,
    render_pass: vk::RenderPass,
    images: &[vk::Image],
    extent: vk::Extent2D,
    format: vk::Format,
) -> (Vec<vk::Framebuffer>, Vec<vk::ImageView>) {
    let (fbs, ivs): (Vec<_>, Vec<_>) = images
        .iter()
        .map(|image| {
            let view_info = vk::ImageViewCreateInfo::default()
                .image(*image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            let mut image_view = vk::ImageView::null();
            let _ = (fns.create_image_view)(device, &view_info, std::ptr::null(), &mut image_view);

            let fb_info = vk::FramebufferCreateInfo::default()
                .render_pass(render_pass)
                .attachments(std::slice::from_ref(&image_view))
                .width(extent.width)
                .height(extent.height)
                .layers(1);
            let mut fb = vk::Framebuffer::null();
            let _ = (fns.create_framebuffer)(device, &fb_info, std::ptr::null(), &mut fb);

            (fb, image_view)
        })
        .unzip();

    (fbs, ivs)
}
