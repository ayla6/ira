#version 450

layout(location = 0) in vec2 position;
layout(location = 1) in vec2 tex_coord;
layout(location = 2) in vec4 color;

layout(push_constant) uniform PC {
    vec2 screen_size;
    vec2 shape_size;
    float corner_radius;
    uint is_shape;
} pc;

layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec4 v_color;

void main() {
    vec2 clip = vec2(
        position.x / pc.screen_size.x * 2.0 - 1.0,
        position.y / pc.screen_size.y * 2.0 - 1.0
    );
    gl_Position = vec4(clip, 0.0, 1.0);
    v_uv = tex_coord;
    v_color = color;
}
