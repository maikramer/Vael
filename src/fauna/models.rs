//! Faceted silhouettes with articulated gait and flight poses shared by a species.
use super::Species;
use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology, prelude::*};
use std::f32::consts::{PI, TAU};

#[derive(Default)]
struct Shape {
    pos: Vec<[f32; 3]>,
    normal: Vec<[f32; 3]>,
    color: Vec<[f32; 4]>,
}
type Tint = [u8; 3];
const COAL: Tint = [43, 38, 39];
const CREAM: Tint = [232, 216, 183];
const COPPER: Tint = [188, 83, 39];
const FUR: Tint = [103, 89, 82];
const PINK: Tint = [174, 122, 115];
const BLUE: Tint = [45, 65, 83];
const AMBER: Tint = [235, 158, 56];
impl Shape {
    fn face(&mut self, p: [Vec3; 3], inside: Vec3, tint: Tint) {
        let mut n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        if n == Vec3::ZERO {
            return;
        }
        let mut verts = p;
        if n.dot(p[0] - inside) < 0. {
            n = -n;
            verts.swap(1, 2);
        }
        let col = Color::srgb_u8(tint[0], tint[1], tint[2])
            .to_linear()
            .to_f32_array();
        for v in verts {
            self.pos.push(v.to_array());
            self.normal.push(n.to_array());
            self.color.push(col);
        }
    }
    // Tapered shafts with closed ends work for muzzles, tail markings and legs.
    fn shaft(&mut self, a: Vec3, b: Vec3, r: [f32; 2], tint: Tint) {
        let axis = (b - a).normalize();
        let x = axis.any_orthonormal_vector();
        let y = axis.cross(x);
        let mid = (a + b) * 0.5;
        for i in 0..8 {
            let radial = |j| {
                let t = TAU * j as f32 / 8.;
                x * t.cos() + y * t.sin()
            };
            let (u, v) = (radial(i), radial(i + 1));
            let (p, q, s, t) = (a + u * r[0], a + v * r[0], b + u * r[1], b + v * r[1]);
            for f in [[p, s, q], [q, s, t], [a, q, p], [b, s, t]] {
                self.face(f, mid, tint);
            }
        }
    }
    fn ellipsoid(&mut self, center: Vec3, size: Vec3, tint: Tint) {
        let v = |i, j| {
            let a = TAU * i as f32 / 10.;
            let b = PI * (j as f32 / 6. - 0.5);
            center + size * Vec3::new(a.cos() * b.cos(), b.sin(), a.sin() * b.cos())
        };
        for i in 0..10 {
            for j in 0..6 {
                for f in [
                    [v(i, j), v(i + 1, j), v(i, j + 1)],
                    [v(i + 1, j), v(i + 1, j + 1), v(i, j + 1)],
                ] {
                    self.face(f, center, tint);
                }
            }
        }
    }
    fn wing(&mut self, root: Vec3, tip: Vec3, rear: Vec3, tint: Tint) {
        let ridge = (root + tip + rear) / 3. + Vec3::Y * (tip - root).length() * 0.065;
        for f in [[root, tip, ridge], [tip, rear, ridge], [rear, root, ridge]] {
            self.face(f, ridge - Vec3::Y, tint);
        }
    }
}
fn mammal(s: &mut Shape, fox: bool, phase: f32) {
    let body = if fox { COPPER } else { FUR };
    let size = if fox { 1.0 } else { 0.47 };
    let v = |x, y, z| Vec3::new(x, y, z) * size;
    let leg = if fox { 0.30 } else { 0.11 };
    let y = leg + 0.105 + 0.02 * phase.sin().abs();
    s.ellipsoid(v(0., y, 0.), v(0.14, 0.16, 0.35), body);
    s.ellipsoid(v(0., y + 0.09, 0.31), v(0.135, 0.15, 0.17), body);
    s.shaft(
        v(0., y + 0.04, 0.4),
        v(0., y + 0.015, 0.57),
        [0.088 * size, 0.028 * size],
        if fox { CREAM } else { FUR },
    );
    s.ellipsoid(
        v(0., y + 0.015, 0.57),
        v(0.035, 0.03, 0.028),
        if fox { COAL } else { PINK },
    );
    for side in [-1., 1.] {
        s.ellipsoid(
            v(side * 0.104, y + 0.14, 0.408),
            v(0.023, 0.024, 0.028),
            COAL,
        );
        if fox {
            s.shaft(
                v(side * 0.082, y + 0.185, 0.29),
                v(side * 0.13, y + 0.36, 0.26),
                [0.071, 0.004],
                COAL,
            );
            s.wing(
                v(side * 0.047, y + 0.20, 0.35),
                v(side * 0.128, y + 0.325, 0.283),
                v(side * 0.13, y + 0.21, 0.34),
                COPPER,
            );
        } else {
            s.ellipsoid(
                v(side * 0.117, y + 0.20, 0.286),
                v(0.066, 0.077, 0.027),
                PINK,
            );
        }
        for fore in [-1., 1.] {
            let z = fore * 0.204;
            let stride = (phase + if fore * side < 0. { PI } else { 0. }).sin();
            let knee = v(side * 0.094, y - leg * 0.52, z + stride * leg * 0.3);
            let foot = v(
                side * 0.097,
                0.025 + 0.055 * stride.max(0.),
                z + stride * leg * 0.56,
            );
            s.shaft(
                v(side * 0.092, y - 0.04, z),
                knee,
                [0.054 * size, 0.034 * size],
                body,
            );
            s.shaft(
                knee,
                foot,
                [0.035 * size, 0.022 * size],
                if fox { COAL } else { FUR },
            );
            s.ellipsoid(foot, v(0.031, 0.026, 0.056), if fox { COAL } else { PINK });
        }
    }
    let sway = 0.065 * phase.sin();
    let mid = v(sway, y - 0.065, -0.55);
    let end = v(sway * 1.7, y - 0.07, -0.76);
    let thick = if fox { 0.099 } else { 0.026 } * size;
    s.shaft(
        v(0., y + 0.015, -0.26),
        mid,
        [0.045 * size, thick],
        if fox { COPPER } else { PINK },
    );
    s.shaft(
        mid,
        end,
        [thick, 0.01 * size],
        if fox { CREAM } else { PINK },
    );
}
fn flyer(s: &mut Shape, bird: bool, phase: f32) {
    let scale = if bird { 1. } else { 0.30 };
    let v = |x, y, z| Vec3::new(x, y, z) * scale;
    s.ellipsoid(
        Vec3::ZERO,
        v(0.055, 0.05, 0.14),
        if bird { CREAM } else { COAL },
    );
    if bird {
        s.ellipsoid(v(0., 0.032, 0.09), v(0.05, 0.053, 0.075), BLUE);
        s.shaft(
            v(0., 0.02, 0.145),
            v(0., 0.014, 0.196),
            [0.024, 0.002],
            COAL,
        );
    }
    for side in [-1., 1.] {
        let flap = phase.sin() * 0.81;
        let wing = |x: f32, z| v(side * x * flap.cos(), x * flap.sin(), z);
        if bird {
            s.wing(wing(0.03, 0.05), wing(0.29, 0.018), wing(0.13, -0.10), BLUE);
            s.wing(
                wing(0.29, 0.018),
                wing(0.47, -0.18),
                wing(0.13, -0.10),
                BLUE,
            );
            s.wing(
                v(0., 0., -0.075),
                v(side * 0.086, 0.012, -0.25),
                v(side * 0.06, 0., -0.094),
                BLUE,
            );
        } else {
            s.wing(wing(0., 0.06), wing(0.23, 0.19), wing(0.21, -0.07), COAL);
            s.wing(
                wing(0.01, 0.04) + Vec3::Y * 0.003,
                wing(0.19, 0.13) + Vec3::Y * 0.003,
                wing(0.17, -0.058) + Vec3::Y * 0.003,
                AMBER,
            );
            s.wing(wing(0., 0.), wing(0.18, -0.07), wing(0.10, -0.17), AMBER);
        }
    }
}
fn trout(s: &mut Shape, phase: f32) {
    let v = Vec3::new;
    let silver = [154, 172, 151];
    let olive = [68, 87, 68];
    let tail = phase.sin() * 0.09;
    s.ellipsoid(Vec3::ZERO, v(0.072, 0.095, 0.27), silver);
    s.ellipsoid(v(0., 0.046, -0.015), v(0.060, 0.056, 0.232), olive);
    s.shaft(
        v(0., 0., -0.14),
        v(tail * 0.7, 0., -0.30),
        [0.062, 0.024],
        olive,
    );
    for side in [-1., 1.] {
        s.wing(
            v(tail * 0.7, 0., -0.27),
            v(tail, side * 0.095, -0.41),
            v(tail, 0., -0.35),
            olive,
        );
        s.wing(
            v(side * 0.052, -0.018, 0.08),
            v(side * 0.13, -0.055, -0.03),
            v(side * 0.046, -0.03, -0.03),
            silver,
        );
        s.ellipsoid(v(side * 0.043, 0.022, 0.187), Vec3::splat(0.014), COAL);
    }
    s.wing(
        v(0., 0.083, 0.025),
        v(0., 0.16, -0.087),
        v(0., 0.076, -0.13),
        olive,
    );
}
fn hare(s: &mut Shape, phase: f32) {
    let v = Vec3::new;
    let coat = [150, 119, 85];
    let stride = phase.sin();
    s.ellipsoid(v(0., 0.21, -0.04), v(0.13, 0.17, 0.25), coat);
    s.ellipsoid(v(0., 0.37, 0.16), v(0.095, 0.105, 0.12), coat);
    s.ellipsoid(v(0., 0.25, -0.29), v(0.065, 0.062, 0.07), CREAM);
    for side in [-1., 1.] {
        let ear = v(side * 0.067, 0.67, 0.08 - stride.abs() * 0.07);
        s.shaft(v(side * 0.043, 0.43, 0.13), ear, [0.037, 0.018], coat);
        s.ellipsoid(ear, v(0.018, 0.031, 0.019), COAL);
        s.ellipsoid(v(side * 0.078, 0.39, 0.211), Vec3::splat(0.018), COAL);
        s.ellipsoid(v(side * 0.083, 0.155, -0.145), v(0.085, 0.14, 0.15), coat);
        let hock = v(side * 0.10, 0.038, -0.17 - stride * 0.12);
        s.shaft(v(side * 0.09, 0.155, -0.15), hock, [0.051, 0.033], coat);
        s.ellipsoid(hock + Vec3::Z * 0.055, v(0.041, 0.033, 0.112), CREAM);
        s.shaft(
            v(side * 0.07, 0.21, 0.10),
            v(side * 0.08, 0.026, 0.12 + stride * 0.09),
            [0.035, 0.022],
            coat,
        );
    }
}

// Feet are the origin, so the resting pose sits directly on the branch.
pub(super) fn woodland_bird(owl: bool, folded: bool, phase: f32) -> Mesh {
    let mut s = Shape::default();
    let v = Vec3::new;
    let coat = if owl { [115, 87, 59] } else { [179, 136, 112] };
    let feather = if owl { [86, 63, 49] } else { [45, 94, 143] };
    let breast = if owl {
        [221, 197, 150]
    } else {
        [212, 179, 153]
    };
    let length = if owl { 1.25 } else { 1. };
    s.ellipsoid(v(0., 0.16, -0.01), v(0.106, 0.13, 0.20) * length, coat);
    s.ellipsoid(v(0., 0.15, 0.087), v(0.093, 0.10, 0.135), breast);
    s.ellipsoid(v(0., 0.285, 0.10), v(0.104, 0.098, 0.11) * length, coat);
    s.shaft(v(0., 0.27, 0.179), v(0., 0.248, 0.26), [0.031, 0.002], COAL);
    s.wing(
        v(0., 0.175, -0.08),
        v(-0.071, 0.11, -0.35),
        v(0.071, 0.11, -0.35),
        COAL,
    );
    for side in [-1., 1.] {
        if owl {
            s.ellipsoid(v(side * 0.051, 0.30, 0.19), v(0.053, 0.056, 0.026), breast);
            s.ellipsoid(v(side * 0.05, 0.305, 0.212), Vec3::splat(0.026), AMBER);
            s.ellipsoid(v(side * 0.05, 0.305, 0.232), Vec3::splat(0.014), COAL);
        } else {
            // Black moustache and pale cheek of a Eurasian jay.
            s.wing(
                v(side * 0.087, 0.29, 0.172),
                v(side * 0.079, 0.248, 0.169),
                v(side * 0.062, 0.276, 0.195),
                COAL,
            );
            s.ellipsoid(v(side * 0.082, 0.298, 0.155), Vec3::splat(0.017), COAL);
            s.ellipsoid(v(side * 0.092, 0.304, 0.164), Vec3::splat(0.005), CREAM);
        }
        if folded {
            s.wing(
                v(side * 0.08, 0.25, 0.06),
                v(side * 0.08, 0.13, -0.24),
                v(side * 0.129, 0.095, -0.05),
                feather,
            );
            // Coverts and barred primaries break up the folded wing silhouette.
            for i in 0..3 {
                let z = 0.02 - i as f32 * 0.049;
                s.wing(
                    v(side * 0.112, 0.192, z),
                    v(side * 0.135, 0.139, z - 0.038),
                    v(side * 0.124, 0.153, z - 0.048),
                    if owl { breast } else { COAL },
                );
            }
            let toe = v(side * 0.043, 0.012, 0.028);
            s.shaft(v(side * 0.045, 0.09, 0.006), toe, [0.012, 0.01], COAL);
            s.shaft(
                toe,
                toe + v(0.006 * side, -0.01, 0.045),
                [0.01, 0.003],
                COAL,
            );
        } else {
            let flap = phase.sin() * 0.88;
            let wing = |x: f32, z| v(side * x * flap.cos(), 0.19 + x * flap.sin(), z);
            let elbow = wing(0.32 * length, 0.105);
            let edge = wing(0.29 * length, -0.13);
            s.wing(wing(0.04, 0.075), elbow, edge, feather);
            s.wing(wing(0.04, 0.075), edge, wing(0.055, -0.12), coat);
            s.wing(elbow, wing(0.49 * length, -0.14), edge, COAL);
        }
    }
    s.mesh()
}

pub(super) fn pose(kind: Species, phase: f32) -> Mesh {
    let mut s = Shape::default();
    if kind == Species::Hare {
        hare(&mut s, phase)
    } else if kind == Species::Trout {
        trout(&mut s, phase)
    } else if kind.flying() {
        flyer(&mut s, kind == Species::Swallow, phase)
    } else {
        mammal(&mut s, kind == Species::Fox, phase)
    }
    s.mesh()
}
impl Shape {
    fn mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normal)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.color)
    }
}
