//! A scene as a glTF binary (`.glb`): one mesh, its colours carried per vertex.
//!
//! GLB is the format Office recommends for 3D models - PowerPoint, Word and Keynote take
//! it, with the colours a vertex carries - and it is one self-contained file. OBJ and STL
//! drop the colours, and PLY's are ignored by PowerPoint, so a molecule exported as any of
//! those arrives grey.

use super::mesh::Mesh;

/// Encodes a triangle mesh as a GLB file.
///
/// The model is moved so the middle of its box is the origin, which is what a slide turns
/// it about. Colours are linear, as glTF's `COLOR_0` is defined; normals are made unit
/// length, which a validator checks and a viewer assumes.
pub fn encode_glb(mesh: &Mesh, name: &str) -> Vec<u8> {
    let vertices = mesh.vertex_count();
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for point in mesh.positions.chunks_exact(3) {
        for axis in 0..3 {
            low[axis] = low[axis].min(point[axis]);
            high[axis] = high[axis].max(point[axis]);
        }
    }
    if vertices == 0 {
        low = [0.0; 3];
        high = [0.0; 3];
    }
    let middle = [0, 1, 2].map(|axis| (low[axis] + high[axis]) / 2.0);

    let mut bin: Vec<u8> = Vec::with_capacity(vertices * 36 + mesh.indices.len() * 4);
    let floats = |bin: &mut Vec<u8>, values: &mut dyn Iterator<Item = f32>| {
        for value in values {
            bin.extend_from_slice(&value.to_le_bytes());
        }
    };
    let positions_at = bin.len();
    floats(
        &mut bin,
        &mut mesh
            .positions
            .chunks_exact(3)
            .flat_map(|point| [0, 1, 2].map(|axis| point[axis] - middle[axis])),
    );
    let normals_at = bin.len();
    floats(
        &mut bin,
        &mut mesh.normals.chunks_exact(3).flat_map(|normal| {
            let length =
                (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
            if length.is_finite() && length > 1e-6 {
                [normal[0] / length, normal[1] / length, normal[2] / length]
            } else {
                [0.0, 0.0, 1.0]
            }
        }),
    );
    let colors_at = bin.len();
    floats(
        &mut bin,
        &mut mesh.colors.iter().map(|value| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            }
        }),
    );
    let indices_at = bin.len();
    for index in &mesh.indices {
        bin.extend_from_slice(&index.to_le_bytes());
    }
    let indices_length = bin.len() - indices_at;
    while bin.len() % 4 != 0 {
        bin.push(0);
    }

    let view = |offset: usize, length: usize, target: u32| serde_json::json!({ "buffer": 0, "byteOffset": offset, "byteLength": length, "target": target });
    let low_moved = [0, 1, 2].map(|axis| low[axis] - middle[axis]);
    let high_moved = [0, 1, 2].map(|axis| high[axis] - middle[axis]);
    let json = serde_json::json!({
        "asset": { "version": "2.0", "generator": "Geis" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0, "name": name }],
        "meshes": [{
            "name": name,
            "primitives": [{
                "attributes": { "POSITION": 0, "NORMAL": 1, "COLOR_0": 2 },
                "indices": 3,
                "material": 0,
                "mode": 4
            }]
        }],
        "materials": [{
            "name": "molecule",
            "pbrMetallicRoughness": { "baseColorFactor": [1.0, 1.0, 1.0, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.5 },
            "doubleSided": true
        }],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": [
            view(positions_at, normals_at - positions_at, 34962),
            view(normals_at, colors_at - normals_at, 34962),
            view(colors_at, indices_at - colors_at, 34962),
            view(indices_at, indices_length, 34963)
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": vertices, "type": "VEC3", "min": low_moved, "max": high_moved },
            { "bufferView": 1, "componentType": 5126, "count": vertices, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5126, "count": vertices, "type": "VEC3" },
            { "bufferView": 3, "componentType": 5125, "count": mesh.indices.len(), "type": "SCALAR" }
        ]
    });
    let mut json = serde_json::to_vec(&json).unwrap_or_default();
    // Chunks are four-byte aligned: the JSON is padded with spaces, the binary with zeros.
    while json.len() % 4 != 0 {
        json.push(b' ');
    }

    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2_u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> Mesh {
        Mesh {
            positions: vec![10.0, 0.0, 0.0, 12.0, 0.0, 0.0, 10.0, 2.0, 0.0],
            normals: vec![0.0, 0.0, 2.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
            colors: vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            vertex_residue: vec![0, 0, 0],
        }
    }

    #[test]
    fn a_glb_is_a_header_a_json_chunk_and_a_binary_chunk() {
        let file = encode_glb(&triangle(), "test");
        assert_eq!(&file[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(file[4..8].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(file[8..12].try_into().unwrap()) as usize,
            file.len()
        );
        let json_length = u32::from_le_bytes(file[12..16].try_into().unwrap()) as usize;
        assert_eq!(&file[16..20], b"JSON");
        assert_eq!(json_length % 4, 0);
        let json: serde_json::Value = serde_json::from_slice(&file[20..20 + json_length]).unwrap();
        let bin_at = 20 + json_length;
        let bin_length = u32::from_le_bytes(file[bin_at..bin_at + 4].try_into().unwrap()) as usize;
        assert_eq!(&file[bin_at + 4..bin_at + 8], b"BIN\0");
        assert_eq!(
            bin_length,
            json["buffers"][0]["byteLength"].as_u64().unwrap() as usize
        );
        assert_eq!(
            json["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"],
            2
        );
        assert_eq!(json["accessors"][3]["count"], 3);
        // Moved to the middle of its box, and the box said.
        assert_eq!(json["accessors"][0]["min"][0], -1.0);
        assert_eq!(json["accessors"][0]["max"][0], 1.0);
        // The first position, read back from the binary chunk: 10 - 11.
        let first = f32::from_le_bytes(file[bin_at + 8..bin_at + 12].try_into().unwrap());
        assert_eq!(first, -1.0);
        // Normals made unit, and a zero one given a direction.
        let normal_at =
            bin_at + 8 + json["bufferViews"][1]["byteOffset"].as_u64().unwrap() as usize;
        let z = f32::from_le_bytes(file[normal_at + 8..normal_at + 12].try_into().unwrap());
        assert_eq!(z, 1.0);
    }

    #[test]
    fn an_empty_scene_is_still_a_valid_file() {
        let empty = Mesh {
            positions: Vec::new(),
            normals: Vec::new(),
            colors: Vec::new(),
            indices: Vec::new(),
            vertex_residue: Vec::new(),
        };
        let file = encode_glb(&empty, "nothing");
        assert_eq!(
            u32::from_le_bytes(file[8..12].try_into().unwrap()) as usize,
            file.len()
        );
    }
}
