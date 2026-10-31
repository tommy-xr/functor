//! Test-only glb builder: named mesh nodes, each a list of box primitives,
//! optionally skinned to one identity joint. Shared by this crate's unit
//! tests and the desktop GL tests (`#[path]`-included), so it uses std only.

pub struct TestNode {
    pub name: Option<String>,
    pub mesh_name: Option<String>,
    /// One box primitive per `(min, max)`.
    pub boxes: Vec<([f32; 3], [f32; 3])>,
}

impl TestNode {
    pub fn named(name: &str) -> TestNode {
        TestNode {
            name: Some(name.to_string()),
            ..TestNode::unnamed()
        }
    }

    pub fn unnamed() -> TestNode {
        TestNode {
            name: None,
            mesh_name: None,
            boxes: vec![([-0.5; 3], [0.5; 3])],
        }
    }

    #[allow(dead_code)]
    pub fn mesh_name(mut self, name: &str) -> TestNode {
        self.mesh_name = Some(name.to_string());
        self
    }

    #[allow(dead_code)]
    pub fn boxes(mut self, boxes: &[([f32; 3], [f32; 3])]) -> TestNode {
        self.boxes = boxes.to_vec();
        self
    }
}

/// A glb of `nodes` as scene roots. `skinned` adds one joint (`root`, at the
/// origin) that every vertex is fully weighted to.
pub fn glb(nodes: &[TestNode], skinned: bool) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views: Vec<String> = Vec::new();
    let mut accessors: Vec<String> = Vec::new();
    // One view + accessor per data block; returns the accessor index.
    let mut push = |bytes: &[u8], count: usize, ty: &str, component: u32, extra: String| {
        let offset = bin.len();
        bin.extend_from_slice(bytes);
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        views.push(format!(
            r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{}}}"#,
            bytes.len()
        ));
        accessors.push(format!(
            r#"{{"bufferView":{},"componentType":{component},"count":{count},"type":"{ty}"{extra}}}"#,
            views.len() - 1
        ));
        accessors.len() - 1
    };
    let floats = |values: &[f32]| {
        values
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<u8>>()
    };

    let mut meshes = Vec::new();
    for node in nodes {
        let mut primitives = Vec::new();
        for (min, max) in &node.boxes {
            let (positions, normals, indices) = box_geometry(*min, *max);
            let count = positions.len() / 3;
            let fmt = |v: &[f32; 3]| format!("[{},{},{}]", v[0], v[1], v[2]);
            let position = push(
                &floats(&positions),
                count,
                "VEC3",
                5126,
                format!(r#","min":{},"max":{}"#, fmt(min), fmt(max)),
            );
            let normal = push(&floats(&normals), count, "VEC3", 5126, String::new());
            let index_bytes: Vec<u8> = indices.iter().flat_map(|i| i.to_le_bytes()).collect();
            let index = push(&index_bytes, indices.len(), "SCALAR", 5125, String::new());
            let skin = if skinned {
                let joints = push(&vec![0u8; count * 4], count, "VEC4", 5121, String::new());
                let weights: Vec<f32> = (0..count).flat_map(|_| [1.0, 0.0, 0.0, 0.0]).collect();
                let weights = push(&floats(&weights), count, "VEC4", 5126, String::new());
                format!(r#","JOINTS_0":{joints},"WEIGHTS_0":{weights}"#)
            } else {
                String::new()
            };
            primitives.push(format!(
                r#"{{"attributes":{{"POSITION":{position},"NORMAL":{normal}{skin}}},"indices":{index}}}"#
            ));
        }
        meshes.push(format!(
            r#"{{{}"primitives":[{}]}}"#,
            name_field(&node.mesh_name),
            primitives.join(",")
        ));
    }

    let joint = nodes.len();
    let mut node_json: Vec<String> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let skin = if skinned { r#","skin":0"# } else { "" };
            format!(r#"{{{}"mesh":{i}{skin}}}"#, name_field(&node.name))
        })
        .collect();
    let mut roots: Vec<usize> = (0..nodes.len()).collect();
    let mut skins = String::new();
    if skinned {
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let ibm = push(&floats(&identity), 1, "MAT4", 5126, String::new());
        node_json.push(r#"{"name":"root"}"#.to_string());
        roots.push(joint);
        skins = format!(r#","skins":[{{"joints":[{joint}],"inverseBindMatrices":{ibm}}}]"#);
    }

    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":{roots:?}}}],"nodes":[{}],"meshes":[{}]{skins},"accessors":[{}],"bufferViews":[{}],"buffers":[{{"byteLength":{}}}]}}"#,
        node_json.join(","),
        meshes.join(","),
        accessors.join(","),
        views.join(","),
        bin.len()
    );
    let mut json = json.into_bytes();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

fn name_field(name: &Option<String>) -> String {
    name.as_ref()
        .map(|name| format!(r#""name":"{name}","#))
        .unwrap_or_default()
}

/// A box with flat per-face normals (24 vertices), counter-clockwise outward.
fn box_geometry(min: [f32; 3], max: [f32; 3]) -> (Vec<f32>, Vec<f32>, Vec<u32>) {
    let center = [0, 1, 2].map(|i| (min[i] + max[i]) / 2.0);
    let half = [0, 1, 2].map(|i| (max[i] - min[i]) / 2.0);
    // (normal axis, sign, u axis, v axis) with u × v = normal.
    let faces = [
        (0, 1.0, 1, 2),
        (0, -1.0, 2, 1),
        (1, 1.0, 2, 0),
        (1, -1.0, 0, 2),
        (2, 1.0, 0, 1),
        (2, -1.0, 1, 0),
    ];
    let (mut positions, mut normals, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    for (n, sign, u, v) in faces {
        let base = (positions.len() / 3) as u32;
        for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let mut p = center;
            p[n] += sign * half[n];
            p[u] += su * half[u];
            p[v] += sv * half[v];
            positions.extend_from_slice(&p);
            let mut normal = [0.0; 3];
            normal[n] = sign;
            normals.extend_from_slice(&normal);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (positions, normals, indices)
}
