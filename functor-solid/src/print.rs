//! Any triangle soup — a game model, a decoded mesh: open, self-intersecting,
//! paper-thin — as a closed solid of a given height, thickened to print. The
//! pipeline behind `functor export --model` and `Solid.fromMesh`.

use crate::{model, remesh, Mesh};

/// A simplify tolerance (mm) that merges only coplanar triangles.
const LOSSLESS: f64 = 1e-6;

pub struct PrintOptions {
    /// Printed height (mm) of the soup's Z extent.
    pub height: f64,
    /// Nozzle width (mm); every surface is thickened to it.
    pub nozzle: f64,
    /// Remesh voxel (mm); default half the nozzle, coarser if too large.
    pub voxel: Option<f64>,
    /// Decimation tolerance (mm); default merges only coplanar triangles.
    pub simplify: Option<f64>,
    /// Largest remesh grid (points) the default voxel may use; a smaller
    /// grid means a coarser voxel (thicker minimum walls), less memory and time.
    pub max_cells: usize,
}

#[derive(Debug)]
pub struct Printed {
    pub mesh: Mesh,
    /// Triangles before simplifying.
    pub remeshed: usize,
    pub voxel: f64,
    /// Thickening actually applied (mm).
    pub shell: f64,
}

/// `soup` (Z up, any units) scaled to `height` mm, centered, on the bed,
/// remeshed closed with every surface thickened to the nozzle width.
pub fn printable(soup: &Mesh, options: &PrintOptions) -> Result<Printed, String> {
    let positive = |x: f64| x > 0.0 && x.is_finite();
    if !(positive(options.height) && positive(options.nozzle) && options.voxel.is_none_or(positive))
    {
        return Err("height, nozzle and voxel must be positive numbers".to_string());
    }
    let mut soup = referenced(soup);
    model::to_print_frame(&mut soup, options.height)?;

    // Sample at half the nozzle unless that grid would be too large.
    let offset = options.nozzle / 2.0;
    let voxel = match options.voxel {
        Some(v) => v,
        None => remesh::fitting_voxel(&soup, options.nozzle / 2.0, offset, options.max_cells),
    };
    // The shell grows the model on every side, so shrink it by that first;
    // the final rescale below then only corrects voxel rounding and leaves
    // walls at full width.
    let shell = remesh::shell(voxel, offset);
    if 2.0 * shell >= options.height {
        return Err(format!(
            "the height must be more than twice the {shell:.2} mm shell"
        ));
    }
    model::to_print_frame(&mut soup, options.height - 2.0 * shell)?;
    let dense = remesh::printable(&soup, voxel, offset)?;
    // Lossless by default: merge only coplanar triangles (flat faces drop a
    // lot; nothing moves). Any real tolerance can self-intersect thin regions,
    // which slicers tolerate but strict CAD checks reject — so it's opt-in.
    let mut mesh = crate::simplify(&dense, options.simplify.unwrap_or(LOSSLESS))?;
    model::to_print_frame(&mut mesh, options.height)?;
    Ok(Printed {
        mesh,
        remeshed: dense.triangles.len(),
        voxel,
        shell,
    })
}

/// `soup` without vertices no triangle uses — they would skew its size.
fn referenced(soup: &Mesh) -> Mesh {
    let mut map = vec![u32::MAX; soup.vertices.len()];
    let mut out = Mesh::default();
    for t in &soup.triangles {
        out.triangles.push(t.map(|i| {
            if map[i as usize] == u32::MAX {
                map[i as usize] = out.vertices.len() as u32;
                out.vertices.push(soup.vertices[i as usize]);
            }
            map[i as usize]
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit box missing its top (open edges, like many game meshes).
    fn open_box() -> Mesh {
        let mut mesh = crate::mesh::boxed([1.0, 1.0, 1.0]);
        mesh.triangles
            .retain(|t| t.iter().any(|&i| mesh.vertices[i as usize][2] < 0.0));
        mesh
    }

    #[test]
    fn an_open_soup_prints_closed_at_its_height() {
        let soup = open_box();
        assert!(soup.non_manifold_edges() > 0);
        let printed = printable(
            &soup,
            &PrintOptions {
                height: 20.0,
                nozzle: 0.8,
                voxel: None,
                simplify: None,
                max_cells: remesh::MAX_CELLS,
            },
        )
        .unwrap();
        let b = printed.mesh.bounds().unwrap();
        assert!(
            b.min[2].abs() < 1e-9 && (b.max[2] - 20.0).abs() < 1e-9,
            "{b:?}"
        );
        assert_eq!(printed.mesh.non_manifold_edges(), 0);
        assert!((printed.shell - 0.4).abs() < 1e-9);
        assert!(printable(
            &soup,
            &PrintOptions {
                height: 0.5,
                nozzle: 0.8,
                voxel: None,
                simplify: None,
                max_cells: remesh::MAX_CELLS,
            }
        )
        .unwrap_err()
        .contains("shell"));
    }

    /// A stray vertex no triangle uses doesn't change the part.
    #[test]
    fn unused_vertices_are_ignored() {
        let options = |_| PrintOptions {
            height: 20.0,
            nozzle: 0.8,
            voxel: None,
            simplify: None,
            max_cells: remesh::MAX_CELLS,
        };
        let soup = open_box();
        let mut stray = soup.clone();
        stray.vertices.push([0.0, 0.0, 100.0]);
        let (a, b) = (
            printable(&soup, &options(())).unwrap().mesh,
            printable(&stray, &options(())).unwrap().mesh,
        );
        assert_eq!(a, b);
    }
}
