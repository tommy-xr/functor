//! A glTF model's parts — what `Scene.modelParts` selects and the manifest's
//! `Assets.<x>Parts` record names. A part is one mesh node with ALL of its
//! primitives, so a multi-material part toggles as a whole.

use std::collections::{HashMap, HashSet};

/// Every mesh node's unique part name, keyed by node index. A part takes its
/// node's name, else its mesh's, else `mesh`. The first node (in the
/// importer's depth-first scene order) with a name keeps it; later repeats
/// get `_2`, `_3`, … skipping every authored name, so an authored `Body_2`
/// is never renamed and a reload names each part the same.
pub fn part_names(document: &gltf::Document) -> HashMap<usize, String> {
    fn visit<'a>(node: gltf::Node<'a>, order: &mut Vec<(usize, &'a str)>) {
        if let Some(mesh) = node.mesh() {
            if !order.iter().any(|(index, _)| *index == node.index()) {
                let base = [node.name(), mesh.name()]
                    .into_iter()
                    .flatten()
                    .find(|name| !name.is_empty())
                    .unwrap_or("mesh");
                order.push((node.index(), base));
            }
        }
        for child in node.children() {
            visit(child, order);
        }
    }
    let mut order = Vec::new();
    for scene in document.scenes() {
        for node in scene.nodes() {
            visit(node, &mut order);
        }
    }
    let mut used: HashSet<String> = order.iter().map(|(_, base)| base.to_string()).collect();
    let mut claimed: HashSet<&str> = HashSet::new();
    order
        .into_iter()
        .map(|(index, base)| {
            if claimed.insert(base) {
                return (index, base.to_string());
            }
            let mut n = 2;
            let mut name = format!("{base}_{n}");
            while !used.insert(name.clone()) {
                n += 1;
                name = format!("{base}_{n}");
            }
            (index, name)
        })
        .collect()
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
            ["Body", "Body_3", "Body_2", "Hat", "mesh", "mesh_2"]
        );
    }
}
