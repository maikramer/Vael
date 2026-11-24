import bevy_ui_render::ui_vertex_output::UiVertexOutput;

struct MangaPanelUniform {
    // p0 (TL), p1 (TR) em UV da página [0..1]
    p01: vec4<f32>,
    // p2 (BR), p3 (BL) em UV da página [0..1]
    p23: vec4<f32>,
    // min_x, min_y, max_x, max_y do quad em UV da página
    bounds: vec4<f32>,
    // pan_x, pan_y, zoom, tex_aspect
    uv_ctrl: vec4<f32>,
    // reveal (0..1), reveal_mode (0=fade,1=slash_r,2=slash_l,3=slash_d,4=pop), border_px, dim (0..1)
    fx: vec4<f32>,
    // speedlines (0..1), flash (0..1), has_texture (0|1), time
    fx2: vec4<f32>,
    // cor de acento (turquesa do Coro ou âmbar da forja)
    accent: vec4<f32>,
    // cor da tinta do contorno
    ink: vec4<f32>,
};

@group(1) @binding(0)
var<uniform> mat: MangaPanelUniform;
@group(1) @binding(1)
var art_texture: texture_2d<f32>;
@group(1) @binding(2)
var art_sampler: sampler;

fn seg_dist(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let denom = max(dot(ab, ab), 1e-5);
    let h = clamp(dot(ap, ab) / denom, 0.0, 1.0);
    return length(ap - ab * h);
}

fn edge_signed(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let e = b - a;
    let len = max(length(e), 1e-5);
    // UI space: +X direita, +Y baixo; quad horário (TL->TR->BR->BL) => normal exterior = (e.y, -e.x)
    let n = vec2<f32>(e.y, -e.x) / len;
    return dot(p - a, n);
}

fn quad_sdf(p: vec2<f32>, v0: vec2<f32>, v1: vec2<f32>, v2: vec2<f32>, v3: vec2<f32>) -> f32 {
    let d0 = edge_signed(p, v0, v1);
    let d1 = edge_signed(p, v1, v2);
    let d2 = edge_signed(p, v2, v3);
    let d3 = edge_signed(p, v3, v0);
    let max_d = max(max(d0, d1), max(d2, d3));
    if max_d <= 0.0 {
        return max_d;
    }
    let s0 = seg_dist(p, v0, v1);
    let s1 = seg_dist(p, v1, v2);
    let s2 = seg_dist(p, v2, v3);
    let s3 = seg_dist(p, v3, v0);
    return min(min(s0, s1), min(s2, s3));
}

fn hash12(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453123);
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let reveal = clamp(mat.fx.x, 0.0, 1.0);
    if reveal <= 0.001 {
        discard;
    }

    let size = max(in.size, vec2<f32>(1.0, 1.0));
    let p = in.uv * size;

    var v0 = mat.p01.xy * size;
    var v1 = mat.p01.zw * size;
    var v2 = mat.p23.xy * size;
    var v3 = mat.p23.zw * size;

    let mode = mat.fx.y;
    let center = (v0 + v1 + v2 + v3) * 0.25;

    // Modo pop: escala com ligeiro recuo inicial a partir do centro do quadro.
    if mode > 3.5 {
        let s = mix(0.86, 1.0, reveal);
        v0 = center + (v0 - center) * s;
        v1 = center + (v1 - center) * s;
        v2 = center + (v2 - center) * s;
        v3 = center + (v3 - center) * s;
    }

    // SDF exato ao quad em píxeis + ligeira irregularidade de bico de pena no traço.
    let raw_sd = quad_sdf(p, v0, v1, v2, v3);
    let nib = 0.42 * sin((p.x + p.y) * 0.065) + 0.24 * cos((p.x - p.y) * 0.13);
    let sd = raw_sd + nib * 0.35;

    // Revelação por corte diagonal ou fade.
    let box_min = mat.bounds.xy;
    let box_max = mat.bounds.zw;
    let box_span = max(box_max - box_min, vec2<f32>(0.001, 0.001));
    let local = (in.uv - box_min) / box_span;

    var reveal_alpha = 1.0;
    if mode < 0.5 {
        reveal_alpha = reveal;
    } else if mode < 1.5 {
        // slash_right: lâmina diagonal da esquerda para a direita
        let coord = local.x * 0.82 + local.y * 0.18;
        let edge = mix(-0.18, 1.18, reveal);
        reveal_alpha = smoothstep(coord - 0.06, coord + 0.02, edge);
    } else if mode < 2.5 {
        // slash_left: lâmina diagonal da direita para a esquerda
        let coord = (1.0 - local.x) * 0.82 + local.y * 0.18;
        let edge = mix(-0.18, 1.18, reveal);
        reveal_alpha = smoothstep(coord - 0.06, coord + 0.02, edge);
    } else if mode < 3.5 {
        // slash_down: corte diagonal de cima para baixo
        let coord = local.y * 0.82 + local.x * 0.18;
        let edge = mix(-0.18, 1.18, reveal);
        reveal_alpha = smoothstep(coord - 0.06, coord + 0.02, edge);
    } else {
        reveal_alpha = smoothstep(0.0, 0.35, reveal);
    }

    if reveal_alpha <= 0.001 {
        discard;
    }

    let border_px = max(mat.fx.z, 1.5) + nib * 0.45;
    let shadow_px = 14.0;

    // Fora da sombra: descartar cedo.
    if sd > shadow_px {
        discard;
    }

    // Sombra subtil projetada no papel da calha (gutter).
    let shadow_sd = quad_sdf(p - vec2<f32>(4.0, 6.0), v0, v1, v2, v3);
    let shadow_mask = (1.0 - smoothstep(0.0, shadow_px, max(shadow_sd, 0.0))) * 0.26;

    // Amostragem da arte (cover aspect + Ken Burns pan/zoom).
    let box_px = box_span * size;
    let box_aspect = box_px.x / max(box_px.y, 1.0);
    let tex_aspect = max(mat.uv_ctrl.w, 0.1);
    var fit = vec2<f32>(1.0, 1.0);
    if box_aspect > tex_aspect {
        fit.y = tex_aspect / box_aspect;
    } else {
        fit.x = box_aspect / tex_aspect;
    }
    let zoom = max(mat.uv_ctrl.z, 0.5);
    let centered = (local - vec2<f32>(0.5, 0.5)) * fit / zoom + mat.uv_ctrl.xy;
    let art_uv = clamp(centered + vec2<f32>(0.5, 0.5), vec2<f32>(0.002), vec2<f32>(0.998));

    var col: vec3<f32>;
    if mat.fx2.z > 0.5 {
        col = textureSample(art_texture, art_sampler, art_uv).rgb;
    } else {
        // Placeholder de caderno com hachura diagonal limpa caso a imagem ainda não exista.
        let paper = vec3<f32>(0.88, 0.84, 0.75);
        let hatch = smoothstep(0.78, 0.92, abs(sin((p.x + p.y) * 0.16)));
        col = mix(paper, paper * 0.78, hatch * 0.45);
        let ring = smoothstep(0.36, 0.34, length(local - vec2<f32>(0.5, 0.5)));
        col = mix(col, mat.accent.rgb * 0.45 + paper * 0.55, ring * 0.25);
    }

    // Grão fino de papel de caderno (muito subtil para não sujar a tinta).
    let grain = (hash12(floor(p * 0.5)) - 0.5) * 0.032;
    col = clamp(col + vec3<f32>(grain), vec3<f32>(0.0), vec3<f32>(1.0));

    // Linhas de velocidade radiais (mangá) nos quadros de impacto.
    let speed = mat.fx2.x;
    if speed > 0.01 {
        let rc = (local - vec2<f32>(0.5, 0.5)) * vec2<f32>(box_aspect, 1.0);
        let ang = atan2(rc.y, rc.x);
        let rad = length(rc);
        let rays1 = pow(abs(sin(ang * 42.0 + 0.7)), 14.0);
        let rays2 = pow(abs(sin(ang * 71.0 - 1.3)), 18.0);
        let rays = clamp((rays1 + rays2 * 0.75) * smoothstep(0.28, 0.72, rad), 0.0, 1.0);
        col = mix(col, mat.ink.rgb, rays * speed * 0.78);
    }

    // Vinheta interior leve na margem do traço para profundidade.
    let inner_rim = 1.0 - smoothstep(border_px, border_px + 18.0, -sd);
    col = mix(col, col * 0.88, inner_rim * 0.28);

    // Atenuação subtil quando o foco avança para o quadro seguinte da mesma página.
    let dim = clamp(mat.fx.w, 0.0, 0.65);
    col = mix(col, col * vec3<f32>(0.76, 0.74, 0.70), dim);

    // Clarão breve de impacto (respeita A11y na CPU).
    let flash = clamp(mat.fx2.y, 0.0, 1.0);
    col = mix(col, vec3<f32>(0.98, 0.96, 0.91), flash * 0.72);

    // Mistura: interior -> contorno preto de tinta -> sombra exterior.
    let aa = 1.35;
    let inside_art = 1.0 - smoothstep(-border_px - aa, -border_px + aa, sd);
    let inside_quad = 1.0 - smoothstep(-aa, aa, sd);

    let panel_rgb = mix(mat.ink.rgb, col, inside_art);
    let final_rgb = mix(vec3<f32>(0.05, 0.04, 0.04), panel_rgb, inside_quad);
    let final_a = max(inside_quad, shadow_mask * (1.0 - inside_quad)) * reveal_alpha;

    return vec4<f32>(final_rgb, final_a);
}
