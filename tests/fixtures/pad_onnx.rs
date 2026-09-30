//! Hand-encoded ONNX graphs shaped like a PAD model (input `[1, 3, 80, 80]`), used to exercise
//! the PAD output-contract checks (GitHub #214, PAD-09) with a real ONNX Runtime session and no
//! model file on disk. None of these graphs is a real classifier: they only fix the output
//! tensor length the detector observes.

#![allow(
    dead_code,
    clippy::cast_possible_truncation,
    reason = "Shared test fixture: each test binary uses a subset of the builders"
)]

fn varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
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

/// `ValueInfoProto` for a float tensor named `name` with static shape `dims`.
fn float_value_info(name: &str, dims: &[u64]) -> Vec<u8> {
    let mut shape = Vec::new();
    for &dim_value in dims {
        let mut dim = Vec::new();
        field_varint(1, dim_value, &mut dim); // Dimension.dim_value
        field_bytes(1, &dim, &mut shape); // TensorShapeProto.dim
    }
    let mut tensor_type = Vec::new();
    field_varint(1, 1, &mut tensor_type); // Tensor.elem_type = FLOAT
    field_bytes(2, &shape, &mut tensor_type); // Tensor.shape
    let mut type_proto = Vec::new();
    field_bytes(1, &tensor_type, &mut type_proto); // TypeProto.tensor_type
    let mut value_info = Vec::new();
    field_bytes(1, name.as_bytes(), &mut value_info); // ValueInfoProto.name
    field_bytes(2, &type_proto, &mut value_info); // ValueInfoProto.type
    value_info
}

/// `AttributeProto` of type INTS.
fn ints_attribute(name: &str, values: &[u64]) -> Vec<u8> {
    let mut attr = Vec::new();
    field_bytes(1, name.as_bytes(), &mut attr); // AttributeProto.name
    for &v in values {
        field_varint(8, v, &mut attr); // AttributeProto.ints
    }
    field_varint(20, 7, &mut attr); // AttributeProto.type = INTS
    attr
}

/// `AttributeProto` of type INT.
fn int_attribute(name: &str, value: u64) -> Vec<u8> {
    let mut attr = Vec::new();
    field_bytes(1, name.as_bytes(), &mut attr); // AttributeProto.name
    field_varint(3, value, &mut attr); // AttributeProto.i
    field_varint(20, 2, &mut attr); // AttributeProto.type = INT
    attr
}

fn model(op_type: &str, attributes: &[Vec<u8>], output_dims: &[u64]) -> Vec<u8> {
    let mut node = Vec::new();
    field_bytes(1, b"input", &mut node); // NodeProto.input
    field_bytes(2, b"logits", &mut node); // NodeProto.output
    field_bytes(4, op_type.as_bytes(), &mut node); // NodeProto.op_type
    for attr in attributes {
        field_bytes(5, attr, &mut node); // NodeProto.attribute
    }

    let mut graph = Vec::new();
    field_bytes(1, &node, &mut graph); // GraphProto.node
    field_bytes(2, b"soos_pad_shape_fixture", &mut graph); // GraphProto.name
    field_bytes(11, &float_value_info("input", &[1, 3, 80, 80]), &mut graph); // GraphProto.input
    field_bytes(12, &float_value_info("logits", output_dims), &mut graph); // GraphProto.output

    let mut opset = Vec::new();
    field_varint(2, 13, &mut opset); // OperatorSetIdProto.version

    let mut out = Vec::new();
    field_varint(1, 7, &mut out); // ModelProto.ir_version
    field_bytes(7, &graph, &mut out); // ModelProto.graph
    field_bytes(8, &opset, &mut out); // ModelProto.opset_import
    out
}

/// `[1, 3, 80, 80] -> ReduceMean(axes=[2, 3]) -> [1, 3]`: a well-shaped 3-class PAD head.
pub fn three_class_pad_model() -> Vec<u8> {
    model(
        "ReduceMean",
        &[
            ints_attribute("axes", &[2, 3]),
            int_attribute("keepdims", 0),
        ],
        &[1, 3],
    )
}

/// `[1, 3, 80, 80] -> ReduceMean(axes=[1, 2, 3]) -> [1]`: a single-logit head. Softmax over a
/// single logit is always 1.0, so without an output-length check it scores every frame live.
pub fn single_logit_pad_model() -> Vec<u8> {
    model(
        "ReduceMean",
        &[
            ints_attribute("axes", &[1, 2, 3]),
            int_attribute("keepdims", 0),
        ],
        &[1],
    )
}

/// `[1, 3, 80, 80] -> Flatten -> [1, 19200]`: a wrong model whose head is not a class vector.
pub fn flatten_pad_model() -> Vec<u8> {
    model("Flatten", &[], &[1, 19_200])
}
