#version 450

void main() {
    vec2 positions[6] = vec2[6](
        vec2(-1.0, -1.0),
        vec2(-0.8, -1.0),
        vec2(-0.8, -0.8),
        vec2(-1.0, -1.0),
        vec2(-0.8, -0.8),
        vec2(-1.0, -0.8)
    );
    gl_Position = vec4(positions[gl_VertexIndex], 0.0, 1.0);
}
