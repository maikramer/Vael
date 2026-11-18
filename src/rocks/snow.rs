//! Snow follows the stone's own facets. Clip the edge continuously at the
//! retention contour so a close-up does not expose the roof heightmap's grid.
use bevy::{
    asset::RenderAssetUsages,
    math::Affine3A,
    mesh::{PrimitiveTopology, VertexAttributeValues},
    prelude::*,
};

#[derive(Clone, Copy)]
struct Sample {
    point: Vec3,
    normal: Vec3,
    retention: f32,
}

impl Sample {
    fn edge(self, other: Self) -> Self {
        let t = self.retention / (self.retention - other.retention);
        Self {
            point: self.point.lerp(other.point, t),
            normal: self.normal.lerp(other.normal, t).normalize_or(Vec3::Y),
            retention: 0.,
        }
    }
}

pub(crate) struct SnowSkin {
    vertices: Vec<Sample>,
}

impl SnowSkin {
    pub(crate) fn from_mesh(mesh: &Mesh, pose: &GlobalTransform) -> Option<Self> {
        let VertexAttributeValues::Float32x3(points) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)?
        else {
            return None;
        };
        let VertexAttributeValues::Float32x3(normals) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)?
        else {
            return None;
        };
        let normal_matrix = pose.affine().matrix3.inverse().transpose();
        let samples: Vec<_> = points
            .iter()
            .zip(normals)
            .map(|(p, n)| {
                let point = Vec3::from_array(*p);
                let normal = Vec3::from_array(*n);
                let up = (normal_matrix * normal).normalize_or(Vec3::Y).y;
                Sample {
                    point,
                    normal,
                    retention: up - 0.52 + 0.12 * (super::noise(point * 7.) - 0.5),
                }
            })
            .collect();
        let mut vertices = Vec::new();
        let mut indices = mesh.indices()?.iter();
        while let (Some(a), Some(b), Some(c)) = (indices.next(), indices.next(), indices.next()) {
            let triangle = [samples[a], samples[b], samples[c]];
            let mut clipped = [triangle[0]; 4];
            let mut count = 0;
            let mut prev = triangle[2];
            for current in triangle {
                if (prev.retention > 0.) != (current.retention > 0.) {
                    clipped[count] = prev.edge(current);
                    count += 1;
                }
                if current.retention > 0. {
                    clipped[count] = current;
                    count += 1;
                }
                prev = current;
            }
            if count >= 3 {
                vertices.extend_from_slice(&clipped[..3]);
            }
            if count == 4 {
                vertices.extend([clipped[0], clipped[2], clipped[3]]);
            }
        }
        Some(Self { vertices })
    }

    pub(crate) fn mesh(&self, depth: f32, to_local: &Affine3A) -> Mesh {
        let up = to_local.transform_vector3(Vec3::Y);
        let positions: Vec<_> = self
            .vertices
            .iter()
            .map(|s| {
                let t = (s.retention / 0.43).clamp(0., 1.);
                let thickness = t * t * (3. - 2. * t) * depth;
                (s.point + s.normal * 0.0015 + up * thickness).to_array()
            })
            .collect();
        let normals: Vec<_> = self.vertices.iter().map(|s| s.normal.to_array()).collect();
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    }
}
