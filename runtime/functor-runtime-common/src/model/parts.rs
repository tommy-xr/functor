//! A glTF model's parts — what `Scene.modelParts` selects and the manifest's
//! `Assets.<x>Parts` record names. A part is one mesh node with ALL of its
//! primitives, so a multi-material part toggles as a whole.

use std::collections::{HashMap, HashSet};

/// Every mesh node's unique part name, keyed by node index, in the importer's
/// depth-first scene order. A part takes its node's name, else its mesh's,
/// else `mesh`; a repeat gets `_2`, `_3`, … (skipping names already taken),
/// so each part stays addressable and a reload names it the same.
pub fn part_names(document: &gltf::Document) -> HashMap<usize, String> {
    fn visit(
        node: gltf::Node,
        used: &mut HashSet<String>,
        names: &mut HashMap<usize, String>,
    ) {
        if let Some(mesh) = node.mesh() {
            if !names.contains_key(&node.index()) {
                let base = [node.name(), mesh.name()]
                    .into_iter()
                    .flatten()
                    .find(|name| !name.is_empty())
                    .unwrap_or("mesh");
                let mut name = base.to_string();
                let mut n = 2;
                while !used.insert(name.clone()) {
                    name = format!("{base}_{n}");
                    n += 1;
                }
                names.insert(node.index(), name);
            }
        }
        for child in node.children() {
            visit(child, used, names);
        }
    }
    let mut used = HashSet::new();
    let mut names = HashMap::new();
    for scene in document.scenes() {
        for node in scene.nodes() {
            visit(node, &mut used, &mut names);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::super::test_glb::{glb, TestNode};
    use super::*;

    fn names(nodes: &[TestNode]) -> Vec<String> {
        let gltf = gltf::Gltf::from_slice(&glb(nodes, false)).expect("fixture parses");
        let names = part_names(&gltf.document);
        let mut sorted: Vec<(usize, String)> = names.into_iter().collect();
        sorted.sort();
        sorted.into_iter().map(|(_, name)| name).collect()
    }

    #[test]
    fn parts_take_node_then_mesh_names_and_suffix_repeats() {
        assert_eq!(
            names(&[
                TestNode::named("Body"),
                TestNode::named("Body"),
                TestNode::named("Body_2"),
                TestNode::unnamed().mesh_name("Hat"),
                TestNode::unnamed(),
                TestNode::named(""),
            ]),
            ["Body", "Body_2", "Body_2_2", "Hat", "mesh", "mesh_2"]
        );
    }
}
