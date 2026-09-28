//! Minimal glTF 2.0 binary (.glb) writer: just enough for textured, skinned
//! meshes. Written against the public Khronos specification.

use serde_json::{Value, json};

pub const ARRAY_BUFFER: u32 = 34962;
pub const ELEMENT_ARRAY_BUFFER: u32 = 34963;
pub const FLOAT: u32 = 5126;
pub const UNSIGNED_BYTE: u32 = 5121;
pub const UNSIGNED_INT: u32 = 5125;

#[derive(Default)]
pub struct Glb {
    pub bin: Vec<u8>,
    pub views: Vec<Value>,
    pub accessors: Vec<Value>,
}

impl Glb {
    pub fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let mut v = json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len() });
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(v);
        self.views.len() - 1
    }

    fn accessor(
        &mut self,
        view: usize,
        component: u32,
        count: usize,
        ty: &str,
        bounds: Option<(Vec<f32>, Vec<f32>)>,
    ) -> usize {
        let mut a =
            json!({ "bufferView": view, "componentType": component, "count": count, "type": ty });
        if let Some((min, max)) = bounds {
            a["min"] = json!(min);
            a["max"] = json!(max);
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    pub fn floats<const N: usize>(
        &mut self,
        data: &[[f32; N]],
        ty: &str,
        with_bounds: bool,
    ) -> usize {
        let bytes: Vec<u8> = data
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let bounds = with_bounds.then(|| {
            let mut min = vec![f32::INFINITY; N];
            let mut max = vec![f32::NEG_INFINITY; N];
            for v in data {
                for i in 0..N {
                    min[i] = min[i].min(v[i]);
                    max[i] = max[i].max(v[i]);
                }
            }
            (min, max)
        });
        let target = if ty == "MAT4" {
            None
        } else {
            Some(ARRAY_BUFFER)
        };
        let view = self.view(&bytes, target);
        self.accessor(view, FLOAT, data.len(), ty, bounds)
    }

    pub fn joints(&mut self, data: &[[u8; 4]]) -> usize {
        let bytes: Vec<u8> = data.iter().flatten().copied().collect();
        let view = self.view(&bytes, Some(ARRAY_BUFFER));
        self.accessor(view, UNSIGNED_BYTE, data.len(), "VEC4", None)
    }

    pub fn indices(&mut self, data: &[[u32; 3]]) -> usize {
        let bytes: Vec<u8> = data
            .iter()
            .flatten()
            .flat_map(|i| i.to_le_bytes())
            .collect();
        let view = self.view(&bytes, Some(ELEMENT_ARRAY_BUFFER));
        self.accessor(view, UNSIGNED_INT, data.len() * 3, "SCALAR", None)
    }

    /// Serialise with `doc` as the JSON chunk (buffers/views/accessors filled in).
    pub fn finish(mut self, mut doc: Value) -> Vec<u8> {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        doc["asset"] = json!({ "version": "2.0", "generator": "opendmc dmc-cli" });
        doc["buffers"] = json!([{ "byteLength": self.bin.len() }]);
        doc["bufferViews"] = Value::Array(self.views);
        doc["accessors"] = Value::Array(self.accessors);
        let mut js = serde_json::to_vec(&doc).expect("json");
        while !js.len().is_multiple_of(4) {
            js.push(b' ');
        }
        let total = 12 + 8 + js.len() + 8 + self.bin.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(js.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&js);
        out.extend_from_slice(&(self.bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&self.bin);
        out
    }
}
