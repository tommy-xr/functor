//! Binary STL export (millimetres — the unit slicers assume for STL).

use std::io::{self, Write};

use crate::Mesh;

/// Write `mesh` as binary STL: an 80-byte header, the triangle count, then per
/// triangle a unit normal, three vertices (f32 LE), and a zero attribute word.
pub fn write_binary(mesh: &Mesh, out: &mut impl Write) -> io::Result<()> {
    let mut header = [0u8; 80];
    let tag = b"functor binary STL (mm)";
    header[..tag.len()].copy_from_slice(tag);
    out.write_all(&header)?;
    out.write_all(&(mesh.triangles.len() as u32).to_le_bytes())?;
    for &[a, b, c] in &mesh.triangles {
        let [a, b, c] = [a, b, c].map(|i| mesh.vertices[i as usize]);
        for value in normal(a, b, c).into_iter().chain(a).chain(b).chain(c) {
            out.write_all(&(value as f32).to_le_bytes())?;
        }
        out.write_all(&[0, 0])?;
    }
    Ok(())
}

fn normal(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let (u, v) = (sub(b, a), sub(c, a));
    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len == 0.0 {
        return [0.0; 3];
    }
    n.map(|x| x / len)
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
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
