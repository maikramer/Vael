import bevy_ui_render::ui_vertex_output::UiVertexOutput;

struct MangaBubbleUniform {
    // kind (0=speech, 1=shout, 2=whisper, 3=narration), border_px, inset_px, opacity
    params: vec4<f32>,
    // has_tail (0|1), tail_dir_x, tail_dir_y, tail_len_px
    tail: vec4<f32>,
    // preenchimento da bolha
    fill: vec4<f32>,
    // cor do contorno de tinta
    ink: vec4<f32>,
    // acento (aba de canto na narração / destaque)
    accent: vec4<f32>,
};

@group(1) @binding(0)
var<uniform> mat: MangaBubbleUniform;

fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn sd_triangle(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> f32 {
    let e0 = b - a;
    let e1 = c - b;
    let e2 = a - c;
    let v0 = p - a;
    let v1 = p - b;
    let v2 = p - c;
    let pq0 = v0 - e0 * clamp(dot(v0, e0) / max(dot(e0, e0), 1e-5), 0.0, 1.0);
    let pq1 = v1 - e1 * clamp(dot(v1, e1) / max(dot(e1, e1), 1e-5), 0.0, 1.0);
    let pq2 = v2 - e2 * clamp(dot(v2, e2) / max(dot(e2, e2), 1e-5), 0.0, 1.0);
    let s = sign(e0.x * e2.y - e0.y * e2.x);
    let d0 = vec2<f32>(dot(pq0, pq0), s * (v0.x * e0.y - v0.y * e0.x));
    let d1 = vec2<f32>(dot(pq1, pq1), s * (v1.x * e1.y - v1.y * e1.x));
    let d2 = vec2<f32>(dot(pq2, pq2), s * (v2.x * e2.y - v2.y * e2.x));
    let d = min(min(d0, d1), d2);
    return -sqrt(d.x) * sign(d.y);
}

fn sd_ellipse_approx(p: vec2<f32>, ab: vec2<f32>) -> f32 {
    let safe = max(ab, vec2<f32>(1.0, 1.0));
    let q = p / safe;
    let k0 = length(q);
    if k0 < 1e-4 {
        return -min(safe.x, safe.y);
    }
    let k1 = length(q / safe);
    return k0 * (k0 - 1.0) / max(k1, 1e-4);
}

fn bubble_sdf(p: vec2<f32>, half_body: vec2<f32>, kind: f32) -> f32 {
    if kind < 0.5 {
        // Speech: oval orgânico de mangá + cauda pontiaguda contínua.
        let r = min(half_body.x * 0.48, half_body.y * 0.88);
        let body = mix(
            sd_rounded_box(p, half_body, r),
            sd_ellipse_approx(p, half_body * vec2<f32>(1.03, 1.08)),
            0.42
        );
        if mat.tail.x > 0.5 {
            let dir = normalize(mat.tail.yz + vec2<f32>(1e-5, 0.0));
            let perp = vec2<f32>(-dir.y, dir.x);
            // Ponto na fronteira da elipse na direção da cauda.
            let edge_t = 1.0 / max(length(dir / max(half_body, vec2<f32>(1.0))), 1e-4);
            let anchor = dir * (edge_t * 0.82);
            let tip = dir * (edge_t + mat.tail.w);
            let base_w = min(15.0, min(half_body.x, half_body.y) * 0.34);
            let a = anchor + perp * base_w;
            let b = anchor - perp * base_w;
            let tail_d = sd_triangle(p, a, b, tip);
            return min(body, tail_d);
        }
        return body;
    } else if kind < 1.5 {
        // Shout: explosão dentada de mangá (picos radiais irregulares).
        let safe = max(half_body, vec2<f32>(1.0, 1.0));
        let q = p / safe;
        let ang = atan2(q.y, q.x);
        let spikes = 16.0;
        let phase = fract((ang / 6.2831853 + 1.0) * spikes);
        let tooth = 1.0 - abs(phase * 2.0 - 1.0);
        let jitter = 0.78 + 0.22 * sin(ang * 5.0 + 1.1) * cos(ang * 3.0 - 0.6);
        let spike_px = mat.params.z * 0.88 * tooth * jitter;
        return sd_ellipse_approx(p, safe) - spike_px;
    } else if kind < 2.5 {
        // Whisper: oval suave ligeiramente recuado.
        let r = min(half_body.x * 0.5, half_body.y * 0.9);
        return sd_rounded_box(p, half_body, r);
    } else {
        // Narration: cartão chanfrado de caderno de cartógrafo.
        return sd_rounded_box(p, half_body, 3.0);
    }
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let opacity = clamp(mat.params.w, 0.0, 1.0);
    if opacity <= 0.001 {
        discard;
    }

    let size = max(in.size, vec2<f32>(4.0, 4.0));
    let p = (in.uv - vec2<f32>(0.5, 0.5)) * size;
    let kind = mat.params.x;
    let border_px = max(mat.params.y, 1.5);
    let inset = mat.params.z;
    let half_body = max(size * 0.5 - vec2<f32>(inset, inset), vec2<f32>(12.0, 12.0));

    let nib = 0.35 * sin((p.x + p.y) * 0.09) + 0.2 * cos((p.x - p.y) * 0.14);
    let sd = bubble_sdf(p, half_body, kind) + nib * 0.3;

    let shadow_px = 10.0;
    if sd > shadow_px {
        discard;
    }

    let shadow_sd = bubble_sdf(p - vec2<f32>(3.0, 4.5), half_body, kind);
    let shadow_mask = (1.0 - smoothstep(0.0, shadow_px, max(shadow_sd, 0.0))) * 0.28;

    let aa = 1.25;
    let inside = 1.0 - smoothstep(-aa, aa, sd);
    var stroke_w = border_px;

    // Whisper: traço tracejado fino.
    if kind > 1.5 && kind < 2.5 {
        let ang = atan2(p.y, p.x);
        let dash = smoothstep(0.32, 0.42, fract((ang / 6.2831853 + 1.0) * 26.0));
        stroke_w = border_px * mix(0.25, 0.85, dash);
    }

    let inside_fill = 1.0 - smoothstep(-stroke_w - aa, -stroke_w + aa, sd);
    var fill_rgb = mat.fill.rgb;

    // Narração: filete interior + barra fina de acento no bordo esquerdo.
    if kind > 2.5 {
        let inner_rule = abs(sd + border_px + 4.5);
        let rule_mask = (1.0 - smoothstep(0.35, 1.15, inner_rule)) * 0.38;
        fill_rgb = mix(fill_rgb, mat.ink.rgb, rule_mask);
        let left_strip = step(p.x, -half_body.x + border_px + 6.0) * inside_fill;
        fill_rgb = mix(fill_rgb, mat.accent.rgb, left_strip * 0.92);
    }

    let rgb = mix(mat.ink.rgb, fill_rgb, inside_fill);
    let final_rgb = mix(vec3<f32>(0.05, 0.04, 0.04), rgb, inside);
    let alpha = max(inside * mat.fill.a, shadow_mask * (1.0 - inside)) * opacity;
    return vec4<f32>(final_rgb, alpha);
}
