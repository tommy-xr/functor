//! `functor export --html`: one self-contained HTML page that shows a
//! project's parts — a parts list with sizes and volumes, an orbit view and
//! eight orthographic views. Meshes are embedded (no side files), and the
//! page's only scripts are pinned CDN builds of three.js, so it works
//! anywhere that serves static HTML, or opened as a local file.

use base64::Engine;
use serde::Serialize;

/// The viewer; `{{TITLE}}` and `{{PARTS}}` are filled in by [`render`].
const TEMPLATE: &str = include_str!("part_page.html");

#[derive(Serialize)]
struct Page<'a> {
    parts: Vec<Part<'a>>,
}

/// One part's mesh, quantized: vertex `i` axis `a` is
/// `min[a] + positions[3i + a] * step[a]` mm (u16, ≤ step / 2 off), and
/// `indices` holds three per triangle (u16, or u32 past 65 536 vertices).
/// Both are little-endian, base64.
#[derive(Serialize)]
struct Part<'a> {
    name: &'a str,
    size: [f64; 3],
    volume: f64,
    triangles: usize,
    min: [f64; 3],
    step: [f64; 3],
    positions: String,
    indices: String,
    index32: bool,
}

/// The page for `parts` (name, mesh in mm, Z up), titled `title`.
pub fn render(title: &str, parts: &[(&str, &functor_solid::Mesh)]) -> Result<String, String> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let parts = parts
        .iter()
        .map(|&(name, mesh)| {
            let b = mesh
                .bounds()
                .filter(|_| !mesh.triangles.is_empty())
                .ok_or_else(|| format!("`{name}` is empty: there is nothing to show"))?;
            let size: [f64; 3] = std::array::from_fn(|a| b.max[a] - b.min[a]);
            let step = size.map(|s| s / f64::from(u16::MAX));
            let mut positions = Vec::with_capacity(mesh.vertices.len() * 6);
            for v in &mesh.vertices {
                for a in 0..3 {
                    let q = if step[a] > 0.0 {
                        ((v[a] - b.min[a]) / step[a]).round() as u16
                    } else {
                        0
                    };
                    positions.extend(q.to_le_bytes());
                }
            }
            let index32 = mesh.vertices.len() > 1 << 16;
            let mut indices =
                Vec::with_capacity(mesh.triangles.len() * if index32 { 12 } else { 6 });
            for &i in mesh.triangles.iter().flatten() {
                if index32 {
                    indices.extend(i.to_le_bytes());
                } else {
                    indices.extend((i as u16).to_le_bytes());
                }
            }
            Ok(Part {
                name,
                size,
                volume: mesh.volume(),
                triangles: mesh.triangles.len(),
                min: b.min,
                step,
                positions: b64.encode(positions),
                indices: b64.encode(indices),
                index32,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let json = serde_json::to_string(&Page { parts })
        .map_err(|e| e.to_string())?
        // Inside <script>: `</script>` in a string would end the element.
        .replace("</", "<\\/");
    // Parts first: base64 and part names cannot contain `{{TITLE}}`.
    Ok(TEMPLATE
        .replace("{{PARTS}}", &json)
        .replace("{{TITLE}}", &escape_html(title)))
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded JSON, decoded back to (name, vertices in mm, triangles).
    fn decode(page: &str) -> Vec<(String, Vec<[f64; 3]>, Vec<[u32; 3]>)> {
        let start = page
            .find(r#"<script id="parts" type="application/json">"#)
            .unwrap();
        let body = &page[start..];
        let body = &body[body.find('>').unwrap() + 1..body.find("</script>").unwrap()];
        let data: serde_json::Value = serde_json::from_str(body).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD;
        let num = |v: &serde_json::Value| v.as_f64().unwrap();
        data["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let pos = b64.decode(p["positions"].as_str().unwrap()).unwrap();
                let idx = b64.decode(p["indices"].as_str().unwrap()).unwrap();
                let verts = pos
                    .chunks(6)
                    .map(|c| {
                        std::array::from_fn(|a| {
                            let q = u16::from_le_bytes([c[2 * a], c[2 * a + 1]]);
                            num(&p["min"][a]) + f64::from(q) * num(&p["step"][a])
                        })
                    })
                    .collect();
                let flat: Vec<u32> = if p["index32"].as_bool().unwrap() {
                    idx.chunks(4)
                        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                        .collect()
                } else {
                    idx.chunks(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]).into())
                        .collect()
                };
                let tris = flat.chunks(3).map(|t| [t[0], t[1], t[2]]).collect();
                (p["name"].as_str().unwrap().to_string(), verts, tris)
            })
            .collect()
    }

    #[test]
    fn meshes_round_trip_within_a_quantum() {
        let cube = functor_solid::Solid::Box {
            size: [30.0, 20.0, 10.0],
        }
        .mesh()
        .unwrap();
        let page = render("box <&>", &[("block", &cube)]).unwrap();
        assert!(page.contains("<title>box &lt;&amp;&gt;</title>"));
        let [(name, verts, tris)] = &decode(&page)[..] else {
            panic!("one part")
        };
        assert_eq!(name, "block");
        assert_eq!(tris, &cube.triangles);
        for (got, want) in verts.iter().zip(&cube.vertices) {
            for a in 0..3 {
                assert!(
                    (got[a] - want[a]).abs() <= 30.0 / 65535.0,
                    "{got:?} vs {want:?}"
                );
            }
        }
    }

    #[test]
    fn large_meshes_use_32_bit_indices() {
        let mesh = functor_solid::Mesh {
            vertices: (0..70_000).map(|i| [i as f64, 0.0, 0.0]).collect(),
            triangles: vec![[0, 1, 69_999]],
        };
        let page = render("big", &[("strip", &mesh)]).unwrap();
        assert_eq!(decode(&page)[0].2, vec![[0, 1, 69_999]]);
    }

    #[test]
    fn empty_parts_are_refused() {
        let err = render("x", &[("nothing", &functor_solid::Mesh::default())]).unwrap_err();
        assert!(err.contains("`nothing` is empty"), "{err}");
    }
}
