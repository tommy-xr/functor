//! Binary STL export (millimetres — the unit slicers assume for STL).

use std::io::{self, Write};

use cgmath::{InnerSpace, Vector3};

use crate::Mesh;

/// Write `mesh` as binary STL: an 80-byte header, the triangle count, then per
/// triangle a unit normal, three vertices (f32 LE), and a zero attribute word.
pub fn write_binary(mesh: &Mesh, out: &mut impl Write) -> io::Result<()> {
    let mut header = [0u8; 80];
    let tag = b"functor binary STL (mm)";
    header[..tag.len()].copy_from_slice(tag);
    out.write_all(&header)?;
    out.write_all(&(mesh.triangles.len() as u32).to_le_bytes())?;
    for tri in &mesh.triangles {
        let [a, b, c] = tri.map(|i| Vector3::from(mesh.vertices[i as usize]));
        let n = (b - a).cross(c - a);
        let n = if n.magnitude2() > 0.0 {
            n.normalize()
        } else {
            n
        };
        for v in [n, a, b, c] {
            for value in [v.x, v.y, v.z] {
                out.write_all(&(value as f32).to_le_bytes())?;
            }
        }
        out.write_all(&[0, 0])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Solid;

    #[test]
    fn cube_stl_has_expected_size_and_outward_normals() {
        let mut bytes = Vec::new();
        write_binary(&Solid::Box { size: [10.0; 3] }.mesh(), &mut bytes).unwrap();
        assert_eq!(bytes.len(), 84 + 12 * 50);
        assert_eq!(u32::from_le_bytes(bytes[80..84].try_into().unwrap()), 12);
        // First triangle is the -z face: normal (0, 0, -1).
        let f = |o: usize| f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        assert_eq!((f(84), f(88), f(92)), (0.0, 0.0, -1.0));
    }
}
