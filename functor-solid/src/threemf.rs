//! 3MF export: one object made of named parts, each on its own filament (part
//! `i` → filament `i + 1`), so a multi-colour print opens ready to slice.
//!
//! Core 3MF carries the geometry; part names and filaments live in
//! `Metadata/model_settings.config` beside a production-extension object file —
//! the layout Bambu Studio reads them from. Output is deterministic: fixed
//! UUIDs, zip timestamps, and number formatting.

use std::fmt::Write as _;
use std::io::{self, Write};

use flate2::write::DeflateEncoder;
use flate2::Compression;

use crate::Mesh;

const NAMESPACES: &str = "xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\" \
xmlns:BambuStudio=\"http://schemas.bambulab.com/package/2021\" \
xmlns:p=\"http://schemas.microsoft.com/3dmanufacturing/production/2015/06\" requiredextensions=\"p\"";
const MODEL_REL: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";
const OBJECTS: &str = "3D/Objects/object_1.model";
/// The part assembly is centered here: the middle of a 256 mm bed.
const BED_CENTER: [f64; 2] = [128.0, 128.0];

/// Write `parts` (name, mesh in mm) as one 3MF object named `name`.
pub fn write(name: &str, parts: &[(&str, &Mesh)], out: &mut impl Write) -> io::Result<()> {
    let assembly = parts.len() + 1;
    let mut objects = String::new();
    let mut components = String::new();
    let mut settings = String::new();
    for (i, (part, mesh)) in parts.iter().enumerate() {
        let id = i + 1;
        let _ = write!(
            objects,
            "<object id=\"{id}\" p:UUID=\"{}\" type=\"model\">{}</object>",
            uuid(1, id),
            mesh_xml(mesh)
        );
        let _ = write!(
            components,
            "<component p:path=\"/{OBJECTS}\" objectid=\"{id}\" p:UUID=\"{}\" \
             transform=\"1 0 0 0 1 0 0 0 1 0 0 0\"/>",
            uuid(2, id)
        );
        let _ = write!(
            settings,
            "<part id=\"{id}\" subtype=\"normal_part\"><metadata key=\"name\" value=\"{}\"/>\
             <metadata key=\"matrix\" value=\"1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1\"/>\
             <metadata key=\"extruder\" value=\"{id}\"/></part>",
            escape(part)
        );
    }
    let [cx, cy] = center_offset(parts);
    let model = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" xml:lang=\"en-US\" {NAMESPACES}>\
         <metadata name=\"Application\">Functor</metadata>\
         <metadata name=\"BambuStudio:3mfVersion\">1</metadata>\
         <resources><object id=\"{assembly}\" p:UUID=\"{}\" type=\"model\"><components>{components}</components></object></resources>\
         <build p:UUID=\"{}\"><item objectid=\"{assembly}\" p:UUID=\"{}\" \
         transform=\"1 0 0 0 1 0 0 0 1 {cx} {cy} 0\" printable=\"1\"/></build></model>",
        uuid(3, 0),
        uuid(4, 0),
        uuid(5, 0)
    );
    let object_file = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" xml:lang=\"en-US\" {NAMESPACES}>\
         <metadata name=\"BambuStudio:3mfVersion\">1</metadata><resources>{objects}</resources><build/></model>"
    );
    let config = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config><object id=\"{assembly}\">\
         <metadata key=\"name\" value=\"{}\"/><metadata key=\"extruder\" value=\"1\"/>{settings}</object>\
         <plate><metadata key=\"plater_id\" value=\"1\"/><model_instance>\
         <metadata key=\"object_id\" value=\"{assembly}\"/><metadata key=\"instance_id\" value=\"0\"/>\
         </model_instance></plate></config>",
        escape(name)
    );
    let mut zip = Zip::default();
    zip.add("[Content_Types].xml", CONTENT_TYPES.as_bytes())?;
    zip.add("_rels/.rels", rels("/3D/3dmodel.model").as_bytes())?;
    zip.add("3D/3dmodel.model", model.as_bytes())?;
    zip.add(
        "3D/_rels/3dmodel.model.rels",
        rels(&format!("/{OBJECTS}")).as_bytes(),
    )?;
    zip.add(OBJECTS, object_file.as_bytes())?;
    zip.add("Metadata/model_settings.config", config.as_bytes())?;
    out.write_all(&zip.finish())
}

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\
<Default Extension=\"config\" ContentType=\"text/xml\"/></Types>";

fn rels(target: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Target=\"{target}\" Id=\"rel-1\" Type=\"{MODEL_REL}\"/></Relationships>"
    )
}

/// A fixed, well-formed UUID per (role, index) — deterministic output.
fn uuid(role: u32, index: usize) -> String {
    format!("{role:08x}-0000-4000-8000-{index:012x}")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn mesh_xml(mesh: &Mesh) -> String {
    let mut xml = String::from("<mesh><vertices>");
    for &[x, y, z] in &mesh.vertices {
        let _ = write!(
            xml,
            "<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>",
            x as f32, y as f32, z as f32
        );
    }
    xml.push_str("</vertices><triangles>");
    for [a, b, c] in &mesh.triangles {
        let _ = write!(xml, "<triangle v1=\"{a}\" v2=\"{b}\" v3=\"{c}\"/>");
    }
    xml.push_str("</triangles></mesh>");
    xml
}

/// Translation that centers the parts' combined XY bounds on [`BED_CENTER`].
fn center_offset(parts: &[(&str, &Mesh)]) -> [f64; 2] {
    let bounds = parts.iter().filter_map(|(_, m)| m.bounds());
    let (lo, hi) = bounds.fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), b| {
        (
            [lo[0].min(b.min[0]), lo[1].min(b.min[1])],
            [hi[0].max(b.max[0]), hi[1].max(b.max[1])],
        )
    });
    if lo[0] > hi[0] {
        return BED_CENTER;
    }
    [0, 1].map(|i| BED_CENTER[i] - (lo[i] + hi[i]) / 2.0)
}

/// A minimal deflate zip writer: local headers, central directory, end record.
#[derive(Default)]
struct Zip {
    data: Vec<u8>,
    directory: Vec<u8>,
    count: u16,
}

impl Zip {
    fn add(&mut self, name: &str, contents: &[u8]) -> io::Result<()> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(contents)?;
        let compressed = encoder.finish()?;
        let crc = crc32fast::hash(contents);
        let offset = self.data.len() as u32;
        // Shared header fields: version 2.0, no flags, deflate, 1980-01-01 00:00.
        let fields = |buf: &mut Vec<u8>| {
            for v in [20u16, 0, 8, 0, 0x21] {
                buf.extend(v.to_le_bytes());
            }
            buf.extend(crc.to_le_bytes());
            buf.extend((compressed.len() as u32).to_le_bytes());
            buf.extend((contents.len() as u32).to_le_bytes());
            buf.extend((name.len() as u16).to_le_bytes());
            buf.extend(0u16.to_le_bytes());
        };
        self.data.extend(0x04034b50u32.to_le_bytes());
        fields(&mut self.data);
        self.data.extend(name.as_bytes());
        self.data.extend(&compressed);

        self.directory.extend(0x02014b50u32.to_le_bytes());
        self.directory.extend(20u16.to_le_bytes()); // made by
        fields(&mut self.directory);
        // Comment length, disk, internal and external attributes.
        self.directory.extend([0u8; 10]);
        self.directory.extend(offset.to_le_bytes());
        self.directory.extend(name.as_bytes());
        self.count += 1;
        Ok(())
    }

    fn finish(mut self) -> Vec<u8> {
        let (dir_offset, dir_len) = (self.data.len() as u32, self.directory.len() as u32);
        self.data.append(&mut self.directory);
        self.data.extend(0x06054b50u32.to_le_bytes());
        self.data.extend([0u8; 4]); // disk numbers
        self.data.extend(self.count.to_le_bytes());
        self.data.extend(self.count.to_le_bytes());
        self.data.extend(dir_len.to_le_bytes());
        self.data.extend(dir_offset.to_le_bytes());
        self.data.extend(0u16.to_le_bytes()); // comment length
        self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Solid;

    #[test]
    fn writes_a_deterministic_two_part_package() {
        let base = Solid::Box { size: [10.0; 3] }.mesh();
        let top = Solid::Sphere { radius: 2.0 }.mesh();
        let mut a = Vec::new();
        write("thing", &[("base", &base), ("top", &top)], &mut a).unwrap();
        let mut b = Vec::new();
        write("thing", &[("base", &base), ("top", &top)], &mut b).unwrap();
        assert_eq!(a, b);
        assert_eq!(&a[..4], b"PK\x03\x04");
        // End record: six entries, and the central directory it points at.
        let end = a.len() - 22;
        assert_eq!(&a[end..end + 4], &0x06054b50u32.to_le_bytes());
        assert_eq!(u16::from_le_bytes([a[end + 10], a[end + 11]]), 6);
        let dir = u32::from_le_bytes(a[end + 16..end + 20].try_into().unwrap()) as usize;
        assert_eq!(&a[dir..dir + 4], &0x02014b50u32.to_le_bytes());
    }
}
