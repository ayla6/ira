#version 450

layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec4 v_color;

layout(binding = 0) uniform sampler2D font_texture;

layout(push_constant) uniform PC {
    vec2 screen_size;
    vec2 shape_size;
    float corner_radius;
    uint is_shape;
} pc;

layout(location = 0) out vec4 out_color;

void main() {
    if (pc.is_shape == 0u) {
        out_color = texture(font_texture, v_uv) * v_color;
    } else if (pc.is_shape == 1u) {
        vec2 half_size = pc.shape_size * 0.5;
        vec2 p = v_uv * pc.shape_size - half_size;
        vec2 d = abs(p) - (half_size - vec2(pc.corner_radius));
        float dist = length(max(d, 0.0)) + min(max(d.x, d.y), 0.0) - pc.corner_radius;
        float alpha = 1.0 - smoothstep(-1.0, 1.0, dist);
        out_color = vec4(v_color.rgb * alpha, v_color.a * alpha);
    } else {
        out_color = v_color;
    }
}
