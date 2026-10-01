//! Hand-encoded ONNX graphs shaped like an embedding extractor (input `[1, 112, 112, 3]` NHWC or
//! `[1, 3, 112, 112]` NCHW), used to exercise the embedding I/O contract (GitHub #268, VIS-14)
//! with a real ONNX Runtime session and no model file on disk. None of these graphs is a real
//! face recognizer: they only fix the input layout and the output length the extractor observes.

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

fn field_f32(field: u64, value: f32, out: &mut Vec<u8>) {
    varint((field << 3) | 5, out);
    out.extend_from_slice(&value.to_le_bytes());
}

/// `ValueInfoProto` for a float tensor named `name` with static shape `dims` (empty = scalar).
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

/// `AttributeProto` of type FLOAT.
fn float_attribute(name: &str, value: f32) -> Vec<u8> {
    let mut attr = Vec::new();
    field_bytes(1, name.as_bytes(), &mut attr); // AttributeProto.name
    field_f32(2, value, &mut attr); // AttributeProto.f
    field_varint(20, 1, &mut attr); // AttributeProto.type = FLOAT
    attr
}

fn node(inputs: &[&str], output: &str, op_type: &str, attributes: &[Vec<u8>]) -> Vec<u8> {
    let mut node = Vec::new();
    for input in inputs {
        field_bytes(1, input.as_bytes(), &mut node); // NodeProto.input
    }
    field_bytes(2, output.as_bytes(), &mut node); // NodeProto.output
    field_bytes(4, op_type.as_bytes(), &mut node); // NodeProto.op_type
    for attr in attributes {
        field_bytes(5, attr, &mut node); // NodeProto.attribute
    }
    node
}

fn model(
    nodes: &[Vec<u8>],
    input: Option<(&str, &[u64])>,
    output: (&str, &[u64]),
    opset: u64,
) -> Vec<u8> {
    let mut graph = Vec::new();
    for n in nodes {
        field_bytes(1, n, &mut graph); // GraphProto.node
    }
    field_bytes(2, b"soos_embedding_shape_fixture", &mut graph); // GraphProto.name
    if let Some((name, dims)) = input {
        field_bytes(11, &float_value_info(name, dims), &mut graph); // GraphProto.input
    }
    field_bytes(12, &float_value_info(output.0, output.1), &mut graph); // GraphProto.output

    let mut opset_proto = Vec::new();
    field_varint(2, opset, &mut opset_proto); // OperatorSetIdProto.version

    let mut out = Vec::new();
    field_varint(1, 7, &mut out); // ModelProto.ir_version
    field_bytes(7, &graph, &mut out); // ModelProto.graph
    field_bytes(8, &opset_proto, &mut out); // ModelProto.opset_import
    out
}

/// `input[dims] -> Flatten(axis=1) -> Slice(axes=[1], starts=[0], ends=[dim]) -> [1, dim]`.
///
/// Opset 9 keeps `Slice` attribute-based, so no initializer tensor is needed.
fn flatten_slice_model(input_dims: &[u64], output_dim: u64) -> Vec<u8> {
    let nodes = [
        node(&["input"], "flat", "Flatten", &[int_attribute("axis", 1)]),
        node(
            &["flat"],
            "embedding",
            "Slice",
            &[
                ints_attribute("axes", &[1]),
                ints_attribute("starts", &[0]),
                ints_attribute("ends", &[output_dim]),
            ],
        ),
    ];
    model(
        &nodes,
        Some(("input", input_dims)),
        ("embedding", &[1, output_dim]),
        9,
    )
}

/// NHWC `[1, 112, 112, 3]` input, `[1, output_dim]` output (the attested ArcFace layout when
/// `output_dim == 512`).
pub fn nhwc_embedding_model(output_dim: u64) -> Vec<u8> {
    flatten_slice_model(&[1, 112, 112, 3], output_dim)
}

/// NCHW `[1, 3, 112, 112]` input, `[1, output_dim]` output.
pub fn nchw_embedding_model(output_dim: u64) -> Vec<u8> {
    flatten_slice_model(&[1, 3, 112, 112], output_dim)
}

/// Rank-2 `[1, 37632]` input: neither NHWC nor NCHW, so the layout cannot be inferred.
pub fn ambiguous_layout_embedding_model() -> Vec<u8> {
    flatten_slice_model(&[1, 37_632], 512)
}

/// A graph with no input at all (`Constant(value_float) -> embedding`): the extractor cannot
/// infer any input layout from it.
pub fn inputless_embedding_model() -> Vec<u8> {
    let nodes = [node(
        &[],
        "embedding",
        "Constant",
        &[float_attribute("value_float", 1.0)],
    )];
    model(&nodes, None, ("embedding", &[]), 13)
}
