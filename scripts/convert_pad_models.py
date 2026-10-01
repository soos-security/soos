#!/usr/bin/env python3
# =============================================================================
# scripts/convert_pad_models.py — Reproducible MiniFASNet .pth -> ONNX conversion
# =============================================================================
# Converts the two upstream Silent-Face-Anti-Spoofing PAD checkpoints to ONNX
# with the upstream model definitions, so that any ONNX file attested in
# models/manifest.toml for PAD can be re-derived from the original weights
# (GitHub #212, PAD-07; walkthrough 161).
#
# Upstream: https://github.com/minivision-ai/Silent-Face-Anti-Spoofing
#   licence Apache-2.0, pinned commit b6d5f04ad78778917853b25c778acef6d5626d15
#   resources/anti_spoof_models/2.7_80x80_MiniFASNetV2.pth
#   resources/anti_spoof_models/4_0_0_80x80_MiniFASNetV1SE.pth
#   src/model_lib/MiniFASNet.py (model definitions)
# Every fetched file is checked against the SHA-256 pinned below before use.
#
# This script is evaluation tooling, never run at install time, and nothing it
# downloads or produces is committed (weights stay outside the repository).
#
# How to rerun (CPU only, throwaway venv outside the repository):
#
#   uv venv --python 3.12 ~/.cache/soos-eval/venv
#   VIRTUAL_ENV=~/.cache/soos-eval/venv uv pip install \
#       --index-strategy unsafe-best-match \
#       --index-url https://download.pytorch.org/whl/cpu \
#       --extra-index-url https://pypi.org/simple \
#       "torch==2.5.1+cpu" "onnx==1.17.0" "onnxruntime==1.20.1" "numpy==2.1.3"
#   ~/.cache/soos-eval/venv/bin/python scripts/convert_pad_models.py \
#       --work-dir ~/.cache/soos-eval/pad --fetch
#
# Exported graph contract (identical to the attested fork export):
#   input  'input'  float32 [batch, 3, 80, 80], NCHW, BGR channel order, raw
#                   pixel values in [0, 255] (upstream ToTensor does NOT divide
#                   by 255: src/data_io/functional.py, "modify by zkx")
#   output 'output' float32 [batch, 3] raw logits (no in-graph softmax),
#                   class order [0 = print attack, 1 = live, 2 = replay attack]
#   opset 11, constant folding on, dynamic batch axis.
#
# The script prints, for each model, the SHA-256 and size of the exported file
# and the max |torch - onnxruntime| softmax difference on fixed seeded inputs.
# The ONNX bytes depend on the exact torch / onnx versions pinned above.
# =============================================================================

import argparse
import hashlib
import importlib.util
import os
import sys
import urllib.request
from collections import OrderedDict

UPSTREAM_COMMIT = "b6d5f04ad78778917853b25c778acef6d5626d15"
UPSTREAM_RAW = (
    "https://raw.githubusercontent.com/minivision-ai/Silent-Face-Anti-Spoofing/"
    + UPSTREAM_COMMIT
    + "/"
)

# Relative upstream path -> pinned SHA-256 of the file at UPSTREAM_COMMIT.
UPSTREAM_FILES = {
    "src/model_lib/MiniFASNet.py":
        "e498c4ec5e1ddfaba62b941a126c19d65aa564999f3309661fe43ee8bf38acd7",
    "resources/anti_spoof_models/2.7_80x80_MiniFASNetV2.pth":
        "a5eb02e1843f19b5386b953cc4c9f011c3f985d0ee2bb9819eea9a142099bec0",
    "resources/anti_spoof_models/4_0_0_80x80_MiniFASNetV1SE.pth":
        "84ee1d37d96894d5e82de5a57df044ef80a58be2b218b5ed7cdfd875ec2f5990",
}

# (checkpoint path, upstream constructor, ONNX output file name)
MODELS = [
    (
        "resources/anti_spoof_models/2.7_80x80_MiniFASNetV2.pth",
        "MiniFASNetV2",
        "2.7_80x80_MiniFASNetV2.onnx",
    ),
    (
        "resources/anti_spoof_models/4_0_0_80x80_MiniFASNetV1SE.pth",
        "MiniFASNetV1SE",
        "4_0_0_80x80_MiniFASNetV1SE.onnx",
    ),
]

INPUT_EDGE = 80
# upstream src/utility.py get_kernel(80, 80) == ((80 + 15) // 16, (80 + 15) // 16)
CONV6_KERNEL = ((INPUT_EDGE + 15) // 16, (INPUT_EDGE + 15) // 16)
OPSET = 11
MAX_FETCH_BYTES = 8 * 1024 * 1024
FETCH_TIMEOUT_S = 120


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fetch(rel_path, dest):
    url = UPSTREAM_RAW + rel_path
    with urllib.request.urlopen(url, timeout=FETCH_TIMEOUT_S) as response:
        data = response.read(MAX_FETCH_BYTES + 1)
    if len(data) > MAX_FETCH_BYTES:
        raise SystemExit(f"refusing {url}: larger than {MAX_FETCH_BYTES} bytes")
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    with open(dest, "wb") as handle:
        handle.write(data)


def verified_upstream_path(work_dir, rel_path, do_fetch):
    dest = os.path.join(work_dir, "upstream", rel_path)
    if do_fetch and not os.path.isfile(dest):
        fetch(rel_path, dest)
    if not os.path.isfile(dest):
        raise SystemExit(f"missing {dest} (rerun with --fetch)")
    actual = sha256_file(dest)
    expected = UPSTREAM_FILES[rel_path]
    if actual != expected:
        raise SystemExit(f"SHA-256 mismatch for {rel_path}: {actual} != {expected}")
    return dest


def load_upstream_module(path):
    spec = importlib.util.spec_from_file_location("upstream_minifasnet", path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot import {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_model(torch, module, constructor, checkpoint):
    model = getattr(module, constructor)(conv6_kernel=CONV6_KERNEL)
    state = torch.load(checkpoint, map_location="cpu", weights_only=True)
    # Upstream checkpoints were saved from DataParallel: strip 'module.' like
    # src/anti_spoof_predict.py does.
    state = OrderedDict(
        (key[7:] if key.startswith("module.") else key, value)
        for key, value in state.items()
    )
    model.load_state_dict(state, strict=True)
    model.eval()
    return model


def export(torch, model, out_path):
    dummy = torch.zeros(1, 3, INPUT_EDGE, INPUT_EDGE)
    torch.onnx.export(
        model,
        dummy,
        out_path,
        export_params=True,
        opset_version=OPSET,
        do_constant_folding=True,
        input_names=["input"],
        output_names=["output"],
        dynamic_axes={"input": {0: "batch_size"}, "output": {0: "batch_size"}},
    )


def parity(torch, numpy, ort, model, out_path):
    rng = numpy.random.default_rng(212)
    batch = rng.uniform(0.0, 255.0, size=(8, 3, INPUT_EDGE, INPUT_EDGE)).astype(numpy.float32)
    session = ort.InferenceSession(out_path, providers=["CPUExecutionProvider"])
    worst = 0.0
    for sample in batch:
        x = sample[None]
        with torch.no_grad():
            ref = torch.softmax(model(torch.from_numpy(x)), dim=1).numpy()
        logits = session.run(None, {"input": x})[0]
        exp = numpy.exp(logits - logits.max(axis=1, keepdims=True))
        got = exp / exp.sum(axis=1, keepdims=True)
        worst = max(worst, float(numpy.abs(ref - got).max()))
    return worst


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work-dir", required=True, help="directory outside the repository")
    parser.add_argument("--fetch", action="store_true", help="download missing upstream files")
    args = parser.parse_args()

    import numpy
    import onnx
    import onnxruntime as ort
    import torch

    torch.manual_seed(0)
    work_dir = os.path.realpath(os.path.expanduser(args.work_dir))
    repo_root = os.path.realpath(os.path.join(os.path.dirname(__file__), ".."))
    if os.path.commonpath([work_dir, repo_root]) == repo_root:
        raise SystemExit("--work-dir must be outside the repository (weights are never committed)")
    source = verified_upstream_path(work_dir, "src/model_lib/MiniFASNet.py", args.fetch)
    module = load_upstream_module(source)
    out_dir = os.path.join(work_dir, "converted")
    os.makedirs(out_dir, exist_ok=True)

    print(f"torch {torch.__version__}, onnx {onnx.__version__}, onnxruntime {ort.__version__}")
    for rel_path, constructor, out_name in MODELS:
        checkpoint = verified_upstream_path(work_dir, rel_path, args.fetch)
        model = load_model(torch, module, constructor, checkpoint)
        out_path = os.path.join(out_dir, out_name)
        export(torch, model, out_path)
        onnx.checker.check_model(onnx.load(out_path))
        worst = parity(torch, numpy, ort, model, out_path)
        print(
            f"{out_name}: sha256 {sha256_file(out_path)} size_bytes {os.path.getsize(out_path)} "
            f"max|torch-ort| softmax {worst:.3e}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
