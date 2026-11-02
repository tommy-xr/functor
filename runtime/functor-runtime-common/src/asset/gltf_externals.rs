//! External resources of a `.gltf` (JSON) model: `.bin` buffers and image files referenced by URI.
//!
//! The model pipeline parses self-contained GLB bytes. A `.gltf` that references sibling files (the
//! common "glTF Separate" export, where several models share one textures folder) is resolved here,
//! in the asset loader's async step: each external file is loaded relative to the `.gltf` (a file path
//! or URL, through the same `load_bytes_async2` as any asset) and everything is packed into one
//! in-memory GLB. Data URIs and anything already embedded are left alone.

use std::collections::HashMap;

use serde_json::{json, Value};

/// True for glTF JSON (as opposed to binary GLB) bytes.
pub fn is_gltf_json(bytes: &[u8]) -> bool {
    let start = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    start.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{')
}

/// The external URIs a glTF document references (buffers and images), in document order, deduplicated.
pub fn external_uris(doc: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for key in ["buffers", "images"] {
        for item in doc.get(key).and_then(Value::as_array).into_iter().flatten() {
            if let Some(uri) = item.get("uri").and_then(Value::as_str) {
                if !uri.starts_with("data:") && !out.iter().any(|u| u == uri) {
                    out.push(uri.to_string());
                }
            }
        }
    }
    out
}

/// Resolve a glTF URI against the `.gltf`'s own path or URL.
///
/// - An absolute URI (`https://...`) is used as is; on a remote `.gltf`, `/x` is relative to its origin.
/// - Otherwise it is relative to the `.gltf`'s directory. File paths are percent-decoded (glTF URIs are
///   URI-encoded: `medical%20kit.bin`) and normalized (`.`/`..` collapsed, as the project-relative path
///   check used on headsets rejects them); URLs stay encoded and are left to the fetcher.
pub fn resolve(gltf_path: &str, uri: &str) -> String {
    if uri.contains("://") {
        return uri.to_string();
    }
    if let Some(scheme_end) = gltf_path.find("://") {
        if uri.starts_with('/') {
            let host_end = gltf_path[scheme_end + 3..]
                .find('/')
                .map_or(gltf_path.len(), |i| scheme_end + 3 + i);
            return format!("{}{uri}", &gltf_path[..host_end]);
        }
        let dir = &gltf_path[..gltf_path.rfind('/').map_or(0, |i| i + 1)];
        return format!("{dir}{uri}");
    }
    let dir = gltf_path.rfind('/').map_or("", |i| &gltf_path[..i]);
    let decoded = percent_decode(uri);
    let joined = if dir.is_empty() {
        decoded
    } else {
        format!("{dir}/{decoded}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            _ => parts.push(seg),
        }
    }
    let root = if joined.starts_with('/') { "/" } else { "" };
    format!("{root}{}", parts.join("/"))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).copied().and_then(hex),
            bytes.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn pad4(v: &mut Vec<u8>, fill: u8) {
    while !v.len().is_multiple_of(4) {
        v.push(fill);
    }
}

fn sniff_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

/// Pack a glTF JSON document and its external resources (`uri -> bytes`) into one GLB.
///
/// Every buffer must be external (a `.gltf`'s buffers always are) and becomes a slice of the one GLB
/// BIN chunk; buffer views are re-pointed with their offsets shifted; external images become buffer
/// views. A referenced resource missing from `resources`, or shorter than its declared `byteLength`
/// (a truncated or mid-write `.bin`), is an error naming it.
pub fn pack_glb(mut doc: Value, resources: &HashMap<String, Vec<u8>>) -> Result<Vec<u8>, String> {
    let mut bin: Vec<u8> = Vec::new();
    // buffer index -> offset of its data inside `bin`
    let mut offsets: Vec<usize> = Vec::new();
    for buffer in doc
        .get("buffers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let uri = buffer.get("uri").and_then(Value::as_str);
        let data = match uri {
            Some(u) if u.starts_with("data:") => {
                return Err(format!(
                    "data-URI buffers are not supported: {}",
                    u.chars().take(40).collect::<String>()
                ))
            }
            Some(u) => resources
                .get(u)
                .ok_or_else(|| format!("missing glTF buffer {u}"))?,
            None => return Err("a .gltf buffer without a uri".to_string()),
        };
        let declared = buffer
            .get("byteLength")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if (data.len() as u64) < declared {
            return Err(format!(
                "buffer {} is {} bytes, declared {declared}",
                uri.unwrap_or(""),
                data.len()
            ));
        }
        pad4(&mut bin, 0);
        offsets.push(bin.len());
        bin.extend_from_slice(data);
    }
    let mut views = doc
        .get("bufferViews")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for view in views.iter_mut() {
        let b = view.get("buffer").and_then(Value::as_u64).unwrap_or(0) as usize;
        let off = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let base = *offsets
            .get(b)
            .ok_or_else(|| format!("bufferView points at missing buffer {b}"))?;
        view["buffer"] = json!(0);
        view["byteOffset"] = json!(base + off);
    }
    if let Some(images) = doc.get_mut("images").and_then(Value::as_array_mut) {
        for image in images.iter_mut() {
            let Some(uri) = image.get("uri").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };
            if uri.starts_with("data:") {
                continue;
            }
            let data = resources
                .get(&uri)
                .ok_or_else(|| format!("missing glTF image {uri}"))?;
            pad4(&mut bin, 0);
            views.push(json!({ "buffer": 0, "byteOffset": bin.len(), "byteLength": data.len() }));
            bin.extend_from_slice(data);
            let obj = image.as_object_mut().expect("glTF image is an object");
            obj.remove("uri");
            obj.insert("bufferView".into(), json!(views.len() - 1));
            if !obj.contains_key("mimeType") {
                obj.insert("mimeType".into(), json!(sniff_mime(data)));
            }
        }
    }
    doc["bufferViews"] = Value::Array(views);
    doc["buffers"] = json!([{ "byteLength": bin.len() }]);

    let mut json_chunk = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
    pad4(&mut json_chunk, b' ');
    pad4(&mut bin, 0);
    let total = 12 + 8 + json_chunk.len() + 8 + bin.len();
    let total_u32 = u32::try_from(total)
        .map_err(|_| format!("packed model is {total} bytes, over GLB's 4 GiB"))?;
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&total_u32.to_le_bytes());
    glb.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json_chunk);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);
    Ok(glb)
}

/// Load a `.gltf`'s external resources relative to `path` and return self-contained GLB bytes. Bytes that
/// are already GLB, or JSON with nothing external, come back unchanged. So does anything that fails (bad
/// JSON, e.g. a file caught mid-write; a missing or truncated resource): the failure is logged and the
/// model pipeline's own fallback (an empty model) applies, so the load still completes and hot reload
/// keeps watching the file.
pub async fn embed_externals(path: &str, bytes: Vec<u8>) -> Vec<u8> {
    match try_embed(path, &bytes).await {
        Ok(Some(glb)) => glb,
        Ok(None) => bytes,
        Err(e) => {
            log::warn!("[model] {path}: {e}");
            bytes
        }
    }
}

async fn try_embed(path: &str, bytes: &[u8]) -> Result<Option<Vec<u8>>, String> {
    if !is_gltf_json(bytes) {
        return Ok(None);
    }
    let doc: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let uris = external_uris(&doc);
    if uris.is_empty() {
        return Ok(None);
    }
    let loads = uris
        .iter()
        .map(|uri| crate::io::load_bytes_async2(resolve(path, uri)));
    let data = futures::future::try_join_all(loads).await?;
    let resources: HashMap<String, Vec<u8>> = uris.into_iter().zip(data).collect();
    pack_glb(doc, &resources).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle_gltf() -> (Value, Vec<u8>) {
        // one triangle, positions only, in an external buffer
        let positions: Vec<u8> = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let doc = json!({
            "asset": { "version": "2.0" },
            "scene": 0, "scenes": [{ "nodes": [0] }], "nodes": [{ "mesh": 0 }],
            "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 }, "material": 0 }] }],
            "materials": [{ "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } } }],
            "textures": [{ "source": 0 }],
            "images": [{ "uri": "../textures/red%20dot.png" }],
            "accessors": [{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                            "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0] }],
            "bufferViews": [{ "buffer": 0, "byteOffset": 0, "byteLength": 36 }],
            "buffers": [{ "uri": "tri.bin", "byteLength": 36 }],
        });
        (doc, positions)
    }

    fn png() -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn detects_json_and_lists_external_uris() {
        assert!(is_gltf_json(b"\xEF\xBB\xBF  {\"asset\":{}}"));
        assert!(!is_gltf_json(b"glTF\x02\0\0\0"));
        let (doc, _) = triangle_gltf();
        assert_eq!(
            external_uris(&doc),
            vec!["tri.bin", "../textures/red%20dot.png"]
        );
    }

    #[test]
    fn resolves_relative_to_the_gltf() {
        assert_eq!(
            resolve("../models/a/gltf/m.gltf", "../textures/red%20dot.png"),
            "../models/a/textures/red dot.png"
        );
        assert_eq!(
            resolve("models/a/m.gltf", "./sub/../m.bin"),
            "models/a/m.bin"
        );
        assert_eq!(resolve("/abs/a/m.gltf", "../t.png"), "/abs/t.png");
        assert_eq!(resolve("m.gltf", "m.bin"), "m.bin");
        assert_eq!(
            resolve("https://h/a/m.gltf", "t%20x.png"),
            "https://h/a/t%20x.png"
        );
        assert_eq!(resolve("https://h/a/m.gltf", "/x.bin"), "https://h/x.bin");
        assert_eq!(
            resolve("models/m.gltf", "https://cdn/x.bin"),
            "https://cdn/x.bin"
        );
    }

    #[test]
    fn packs_a_glb_the_gltf_crate_loads_with_its_image() {
        let (doc, positions) = triangle_gltf();
        let resources = HashMap::from([
            ("tri.bin".to_string(), positions.clone()),
            ("../textures/red%20dot.png".to_string(), png()),
        ]);
        let glb = pack_glb(doc, &resources).unwrap();
        let gltf = gltf::Gltf::from_slice(&glb).unwrap();
        let blob = gltf.blob.as_ref().unwrap();
        let view = gltf.document.accessors().next().unwrap().view().unwrap();
        assert_eq!(
            &blob[view.offset()..view.offset() + view.length()],
            positions.as_slice()
        );
        let image = gltf.document.images().next().unwrap();
        match image.source() {
            gltf::image::Source::View { view, mime_type } => {
                assert_eq!(mime_type, "image/png");
                assert!(image::load_from_memory(
                    &blob[view.offset()..view.offset() + view.length()]
                )
                .is_ok());
            }
            _ => panic!("image was not embedded"),
        }
    }

    #[test]
    fn truncated_buffers_are_rejected() {
        let (doc, positions) = triangle_gltf();
        let resources = HashMap::from([
            ("tri.bin".to_string(), positions[..20].to_vec()),
            ("../textures/red%20dot.png".to_string(), png()),
        ]);
        assert!(pack_glb(doc, &resources)
            .unwrap_err()
            .contains("declared 36"));
    }

    #[test]
    fn failures_pass_the_original_bytes_through() {
        let bad = b"{ not json".to_vec();
        assert_eq!(
            futures::executor::block_on(embed_externals("m.gltf", bad.clone())),
            bad
        );
        let missing = serde_json::to_vec(&triangle_gltf().0).unwrap();
        let out =
            futures::executor::block_on(embed_externals("/nonexistent/m.gltf", missing.clone()));
        assert_eq!(out, missing);
    }

    #[test]
    fn missing_resources_are_named() {
        let (doc, positions) = triangle_gltf();
        let only_bin = HashMap::from([("tri.bin".to_string(), positions)]);
        assert!(pack_glb(doc, &only_bin)
            .unwrap_err()
            .contains("red%20dot.png"));
    }
}
