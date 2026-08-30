#version 450

#extension GL_EXT_samplerless_texture_functions : require

layout(set = 0, binding = 0) uniform texture2D tex;

layout(push_constant, std430) uniform Data {
    ivec2 direction;
    int radius;
    float sigma;
    float normalization;
} data;

layout(location = 0) out vec4 out_color;

ivec2 clamp_position(ivec2 position) {
    return clamp(position, ivec2(0), textureSize(tex, 0) - ivec2(1));
}

void main() {
    ivec2 position = ivec2(gl_FragCoord.xy);
    vec4 color = texelFetch(tex, position, 0) * data.normalization;
    float denominator = 2.0 * data.sigma * data.sigma;
    for (int offset = 1; offset <= data.radius; offset++) {
        float weight = exp(-float(offset * offset) / denominator) * data.normalization;
        ivec2 delta = data.direction * offset;
        color += texelFetch(tex, clamp_position(position - delta), 0) * weight;
        color += texelFetch(tex, clamp_position(position + delta), 0) * weight;
    }
    out_color = color;
}
