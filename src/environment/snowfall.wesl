import bevy_pbr::render::mesh_view_bindings::view;

// bevy_elements::PrecipitationState stores positions in metres (Vael uses 1 unit/m).
struct Particle {
    pos: vec3<f32>,
    kind: u32,
    velocity: vec3<f32>,
    life: f32,
}
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<storage, read> particles: array<Particle>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> daylight: f32;

struct Flake {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) alpha: f32,
}

@vertex
fn vertex(@location(0) corner_sample: vec3<f32>) -> Flake {
    let id = u32(corner_sample.z * f32(arrayLength(&particles)));
    let p = particles[id];
    let distance_m = distance(p.pos, view.world_position);
    if (p.kind & 3u) != 1u || distance_m >= 29.0 {
        return Flake(vec4<f32>(2.0, 2.0, 0.5, 1.0), vec2<f32>(0.0), 0.0);
    }
    let seed = fract(f32(id % 4093u) * 0.618034);
    let phase = seed * 6.28318 + p.life * (0.7 + seed);
    let radius = mix(0.034, 0.095, seed * seed);
    let corner = corner_sample.xy;
    let tilted = corner * vec2<f32>(0.62 + 0.38 * abs(cos(phase)), 1.0);
    let offset = vec2<f32>(cos(phase) * tilted.x - sin(phase) * tilted.y,
                           sin(phase) * tilted.x + cos(phase) * tilted.y);
    let world = p.pos + radius * (view.world_from_view[0].xyz * offset.x + view.world_from_view[1].xyz * offset.y);
    let fade = smoothstep(0.7, 2.4, distance_m) * (1.0 - smoothstep(17.0, 29.0, distance_m));
    return Flake(view.clip_from_world * vec4<f32>(world, 1.0), corner, fade);
}

@fragment
fn fragment(in: Flake) -> @location(0) vec4<f32> {
    // A soft six-lobed silhouette survives at close range. Pixel filtering smooths distant flakes.
    let angle = atan2(in.local.y, in.local.x);
    let d = length(in.local) - (0.77 + 0.10 * cos(6.0 * angle));
    let edge = max(0.12, fwidth(d));
    let coverage = 1.0 - smoothstep(-edge, edge, d);
    let ice = mix(vec3<f32>(0.62, 0.74, 0.88), vec3<f32>(1.05, 1.09, 1.12), 1.0 - smoothstep(-0.55, 0.0, d));
    return vec4<f32>(ice * mix(0.19, 1.0, daylight), coverage * in.alpha * 0.91);
}
