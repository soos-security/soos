//! ORT-owned output tensor wiping contract (GitHub #255, review finding VIS-13).
//!
//! The raw 512-D embedding, SCRFD landmark tensors and PAD logits live in buffers owned by
//! ONNX Runtime (`SessionOutputs`). Copying them into a `Zeroizing` container is not enough:
//! the ORT allocation itself must be overwritten before it is released. `ZeroizingOutputs`
//! wraps the outputs of every production `Session::run` and wipes every `f32` output tensor
//! in place (`try_extract_tensor_mut`) when it is dropped or explicitly wiped.
//!
//! The graph used here is a deterministic in-memory single-node `Identity` model
//! (`x[1,3] -> y[1,3]`), so the contract runs hermetically in CI without any model file.
//! ORT internal activation buffers remain out of reach (documented limitation, ADR VAY).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Test suite utilizes direct assertions and protobuf fixture encoding"
)]

use soos_inference_ort::outputs::ZeroizingOutputs;

mod identity_model {
    fn varint(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }

    fn field_varint(field: u64, value: u64, out: &mut Vec<u8>) {
        varint(field << 3, out);
        varint(value, out);
    }

    fn field_bytes(field: u64, bytes: &[u8], out: &mut Vec<u8>) {
        varint((field << 3) | 2, out);
        varint(bytes.len() as u64, out);
        out.extend_from_slice(bytes);
    }

    fn float_value_info(name: &str) -> Vec<u8> {
        let mut dims = Vec::new();
        for dim_value in [1u64, 3u64] {
            let mut dim = Vec::new();
            field_varint(1, dim_value, &mut dim);
            field_bytes(1, &dim, &mut dims);
        }
        let mut tensor_type = Vec::new();
        field_varint(1, 1, &mut tensor_type); // FLOAT
        field_bytes(2, &dims, &mut tensor_type);
        let mut type_proto = Vec::new();
        field_bytes(1, &tensor_type, &mut type_proto);
        let mut value_info = Vec::new();
        field_bytes(1, name.as_bytes(), &mut value_info);
        field_bytes(2, &type_proto, &mut value_info);
        value_info
    }

    /// Serialized `ModelProto` of a single-node `Identity` graph (`x[1,3] -> y[1,3]`).
    pub fn bytes() -> Vec<u8> {
        let mut node = Vec::new();
        field_bytes(1, b"x", &mut node);
        field_bytes(2, b"y", &mut node);
        field_bytes(4, b"Identity", &mut node);
        let mut graph = Vec::new();
        field_bytes(1, &node, &mut graph);
        field_bytes(2, b"soos_identity", &mut graph);
        field_bytes(11, &float_value_info("x"), &mut graph);
        field_bytes(12, &float_value_info("y"), &mut graph);
        let mut opset = Vec::new();
        field_varint(2, 13, &mut opset);
        let mut model = Vec::new();
        field_varint(1, 7, &mut model);
        field_bytes(7, &graph, &mut model);
        field_bytes(8, &opset, &mut model);
        model
    }
}

fn identity_session() -> ort::session::Session {
    ort::session::Session::builder()
        .expect("ORT session builder")
        .commit_from_memory(&identity_model::bytes())
        .expect("minimal identity ONNX model must load")
}

const SECRET: [f32; 3] = [0.25, -0.5, 0.75];

#[test]
fn test_zeroizing_outputs_exposes_outputs_before_wipe() {
    let mut session = identity_session();
    let input = ort::value::TensorRef::from_array_view(([1usize, 3], SECRET.as_slice()))
        .expect("input tensor");
    let outputs = ZeroizingOutputs::new(session.run(ort::inputs![input]).expect("run"));
    let (_, data) = outputs[0].try_extract_tensor::<f32>().expect("f32 output");
    assert_eq!(data, SECRET.as_slice());
}

#[test]
fn test_zeroizing_outputs_wipes_ort_owned_output_tensor() {
    let mut session = identity_session();
    let input = ort::value::TensorRef::from_array_view(([1usize, 3], SECRET.as_slice()))
        .expect("input tensor");
    let mut outputs = ZeroizingOutputs::new(session.run(ort::inputs![input]).expect("run"));

    let wiped = outputs.wipe();
    assert_eq!(wiped, 1, "exactly one f32 output tensor must be wiped");

    let (_, data) = outputs[0].try_extract_tensor::<f32>().expect("f32 output");
    assert!(
        data.iter().all(|&v| v == 0.0),
        "ORT-owned output buffer must be overwritten in place, got {data:?}"
    );
}

#[test]
fn test_zeroizing_outputs_wipe_is_idempotent() {
    let mut session = identity_session();
    let input = ort::value::TensorRef::from_array_view(([1usize, 3], SECRET.as_slice()))
        .expect("input tensor");
    let mut outputs = ZeroizingOutputs::new(session.run(ort::inputs![input]).expect("run"));
    assert_eq!(outputs.wipe(), 1);
    assert_eq!(outputs.wipe(), 1);
    drop(outputs); // Drop wipes again; must not panic.
}
