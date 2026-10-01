#!/usr/bin/env python3
# =============================================================================
# scripts/compare_pad_models.py — Numerical equivalence check of two PAD ONNX files
# =============================================================================
# Runs a reference and a candidate MiniFASNet ONNX graph with onnxruntime on
# the same inputs and reports the max |softmax difference|, the argmax
# agreement and the argmax histogram, under both input conventions:
#   raw    pixel values in [0, 255] (upstream Silent-Face-Anti-Spoofing ToTensor)
#   scaled pixel / 255.0 in [0, 1]  (soos OrtPadDetector::prepare_input today)
# It also compares the weight initializers of both graphs value by value.
# Used to attest that a shipped or fork ONNX equals the conversion of the
# upstream .pth by scripts/convert_pad_models.py (GitHub #212, walkthrough 161).
#
# Inputs are synthetic (constants, gradients, checkerboards, seeded noise) plus
# optional crops of public-domain / CC0 images from --images-dir (e.g. the
# scikit-image v0.24.0 sample images astronaut, coffee, chelsea, rocket). Never
# point it at personal face data; nothing it reads or prints is committed.
#
# Usage (same venv as scripts/convert_pad_models.py, plus pillow==11.0.0):
#   ~/.cache/soos-eval/venv/bin/python scripts/compare_pad_models.py \
#       --reference ~/.cache/soos-eval/pad/converted/2.7_80x80_MiniFASNetV2.onnx \
#       --candidate /var/lib/soos/models/minifasnet_v2_80x80.onnx \
#       --images-dir ~/.cache/soos-eval/pad/images
# =============================================================================

import argparse
import os
import sys

EDGE = 80
CLASS_NAMES = ["print", "live", "replay"]
CROP_FRACTIONS = [1.0, 0.75, 0.5, 0.33]
CROP_POSITIONS = [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0), (0.0, 1.0), (1.0, 0.0), (0.25, 0.6)]
MAX_IMAGE_EDGE = 8192


def synthetic_inputs(numpy):
    """Fixed synthetic BGR uint8 80x80 images."""
    images = {}
    for level in (0, 64, 128, 192, 255):
        images[f"constant_{level}"] = numpy.full((EDGE, EDGE, 3), level, numpy.uint8)
    ramp = numpy.linspace(0, 255, EDGE).astype(numpy.uint8)
    images["gradient_h"] = numpy.repeat(numpy.tile(ramp, (EDGE, 1))[:, :, None], 3, axis=2)
    images["gradient_v"] = numpy.ascontiguousarray(images["gradient_h"].transpose(1, 0, 2))
    colour = numpy.stack([numpy.tile(ramp, (EDGE, 1)), numpy.tile(ramp, (EDGE, 1)).T,
                          numpy.full((EDGE, EDGE), 96, numpy.uint8)], axis=2)
    images["gradient_colour"] = colour.astype(numpy.uint8)
    for cell in (2, 5, 10, 20):
        yy, xx = numpy.mgrid[0:EDGE, 0:EDGE]
        board = (((yy // cell) + (xx // cell)) % 2 * 255).astype(numpy.uint8)
        images[f"checker_{cell}"] = numpy.repeat(board[:, :, None], 3, axis=2)
    rng = numpy.random.default_rng(161)
    for i in range(32):
        images[f"uniform_noise_{i}"] = rng.integers(0, 256, (EDGE, EDGE, 3), dtype=numpy.uint8)
    for i in range(16):
        coarse = rng.integers(0, 256, (10, 10, 3)).astype(numpy.float32)
        smooth = numpy.kron(coarse, numpy.ones((8, 8, 1), numpy.float32))
        images[f"blocky_noise_{i}"] = smooth.astype(numpy.uint8)
    return images


def natural_inputs(numpy, image_module, images_dir):
    """80x80 BGR crops of every image in images_dir at fixed fractions and positions."""
    images = {}
    if not images_dir:
        return images
    for name in sorted(os.listdir(images_dir)):
        if not name.lower().endswith((".png", ".jpg", ".jpeg")):
            continue
        with image_module.open(os.path.join(images_dir, name)) as img:
            if max(img.size) > MAX_IMAGE_EDGE:
                raise SystemExit(f"{name}: larger than {MAX_IMAGE_EDGE} px")
            rgb = img.convert("RGB")
            width, height = rgb.size
            for fraction in CROP_FRACTIONS:
                side = max(1, int(min(width, height) * fraction))
                for px, py in CROP_POSITIONS:
                    left = int((width - side) * px)
                    top = int((height - side) * py)
                    crop = rgb.crop((left, top, left + side, top + side))
                    crop = crop.resize((EDGE, EDGE), image_module.Resampling.BILINEAR)
                    arr = numpy.asarray(crop, numpy.uint8)[:, :, ::-1]  # RGB -> BGR
                    images[f"{name}_f{fraction}_p{px}-{py}"] = numpy.ascontiguousarray(arr)
    return images


def softmax(numpy, logits):
    shifted = numpy.exp(logits - logits.max(axis=1, keepdims=True))
    return shifted / shifted.sum(axis=1, keepdims=True)


def run(numpy, session, bgr, scale):
    tensor = bgr.astype(numpy.float32).transpose(2, 0, 1)[None] * scale
    name = session.get_inputs()[0].name
    return softmax(numpy, session.run(None, {name: tensor})[0])[0]


def compare_initializers(numpy, onnx, reference, candidate):
    from onnx import numpy_helper

    def tensors(path):
        graph = onnx.load(path).graph
        arrays = [numpy_helper.to_array(t).astype(numpy.float64) for t in graph.initializer]
        return sorted(arrays, key=lambda a: (a.shape, float(a.sum()), float(numpy.abs(a).sum())))

    ref, cand = tensors(reference), tensors(candidate)
    if len(ref) != len(cand):
        return f"initializer count differs: {len(ref)} vs {len(cand)}"
    worst = 0.0
    for a, b in zip(ref, cand):
        if a.shape != b.shape:
            return f"initializer shapes differ: {a.shape} vs {b.shape}"
        worst = max(worst, float(numpy.abs(a - b).max()) if a.size else 0.0)
    params = sum(a.size for a in ref)
    return f"{len(ref)} initializers, {params} parameters, max |delta| {worst:.3e}"


def main():
    parser = argparse.ArgumentParser(description="Compare two MiniFASNet ONNX graphs")
    parser.add_argument("--reference", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--images-dir", default=None)
    args = parser.parse_args()

    import numpy
    import onnx
    import onnxruntime as ort
    from PIL import Image

    sessions = [
        ort.InferenceSession(os.path.expanduser(p), providers=["CPUExecutionProvider"])
        for p in (args.reference, args.candidate)
    ]
    for label, session in zip(("reference", "candidate"), sessions):
        i, o = session.get_inputs()[0], session.get_outputs()[0]
        print(f"{label}: input {i.name} {i.shape} {i.type}, output {o.name} {o.shape}")
    print("weights:", compare_initializers(
        numpy, onnx, os.path.expanduser(args.reference), os.path.expanduser(args.candidate)))

    groups = {
        "synthetic": synthetic_inputs(numpy),
        "natural": natural_inputs(numpy, Image, args.images_dir and os.path.expanduser(args.images_dir)),
    }
    for convention, scale in (("raw", 1.0), ("scaled", 1.0 / 255.0)):
        for group, images in groups.items():
            if not images:
                continue
            worst, agree, hist = 0.0, 0, [0, 0, 0]
            live_max = 0.0
            for bgr in images.values():
                ref = run(numpy, sessions[0], bgr, scale)
                cand = run(numpy, sessions[1], bgr, scale)
                worst = max(worst, float(numpy.abs(ref - cand).max()))
                agree += int(ref.argmax() == cand.argmax())
                hist[int(ref.argmax())] += 1
                live_max = max(live_max, float(ref[1]))
            histogram = ", ".join(f"{n}={c}" for n, c in zip(CLASS_NAMES, hist))
            print(
                f"[{convention}] {group}: n={len(images)} max|dsoftmax| {worst:.3e} "
                f"argmax agreement {agree}/{len(images)} reference argmax {{{histogram}}} "
                f"max p(index 1) {live_max:.4f}"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main())
