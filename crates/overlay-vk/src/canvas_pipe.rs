//! Canvas textured-quad pipeline — shaders, vertex layout, blending.
//!
//! Split out of `canvas_res.rs` (which owns texture/sampler/descriptor
//! resources) so neither file exceeds the line cap. Everything here is
//! construction only; the orchestrator in `canvas.rs` owns the handles.

use ash::vk;

use ira_overlay::types::DeviceFns;

const CANVAS_VERT_SPV: &[u8] = include_bytes!("../shaders/canvas_vert.spv");
const CANVAS_FRAG_SPV: &[u8] = include_bytes!("../shaders/canvas_frag.spv");

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
    let (shader_vert, shader_frag) = load_shaders(fns, device);
    let stages = shader_stages(shader_vert, shader_frag);

    let binding = vertex_binding();
    let attributes = vertex_attributes();
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
    let blend_attachment = blend_attachment();
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

unsafe fn load_shaders(fns: DeviceFns, device: vk::Device) -> (vk::ShaderModule, vk::ShaderModule) {
    let vert_code = words(CANVAS_VERT_SPV);
    let vert_info = vk::ShaderModuleCreateInfo::default().code(&vert_code);
    let mut shader_vert = vk::ShaderModule::null();
    let _ = (fns.create_shader_module)(device, &vert_info, std::ptr::null(), &mut shader_vert);

    let frag_code = words(CANVAS_FRAG_SPV);
    let frag_info = vk::ShaderModuleCreateInfo::default().code(&frag_code);
    let mut shader_frag = vk::ShaderModule::null();
    let _ = (fns.create_shader_module)(device, &frag_info, std::ptr::null(), &mut shader_frag);
    (shader_vert, shader_frag)
}

fn shader_stages(
    shader_vert: vk::ShaderModule,
    shader_frag: vk::ShaderModule,
) -> [vk::PipelineShaderStageCreateInfo<'static>; 2] {
    [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(shader_vert)
            .name(c"main"),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(shader_frag)
            .name(c"main"),
    ]
}

fn vertex_binding() -> vk::VertexInputBindingDescription {
    vk::VertexInputBindingDescription::default()
        .binding(0)
        .stride(20)
        .input_rate(vk::VertexInputRate::VERTEX)
}

fn vertex_attributes() -> [vk::VertexInputAttributeDescription; 3] {
    [
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
    ]
}

fn blend_attachment() -> vk::PipelineColorBlendAttachmentState {
    // Canvas pixels are premultiplied — same factors as the glyph pipeline.
    vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA)
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::ONE)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .alpha_blend_op(vk::BlendOp::ADD)
}

fn words(spv: &[u8]) -> Vec<u32> {
    let (chunks, _) = spv.as_chunks::<4>();
    chunks.iter().map(|c| u32::from_le_bytes(*c)).collect()
}
