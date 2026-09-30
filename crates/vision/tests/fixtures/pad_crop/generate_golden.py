#!/usr/bin/env python3
"""Generates the golden PAD context crops used by `pad_crop_geometry_tests.rs` (GitHub #213).

Reference implementation of the Silent-Face-Anti-Spoofing crop, written with NumPy only
(OpenCV is prohibited in this repository and is not needed to reproduce the convention):

* `get_new_box` is a line-by-line transcription of upstream
  `src/generate_patches.py::CropImage._get_new_box` (inclusive `src_w - 1` / `src_h - 1`
  bounds, scale capped by `(src - 1) / box`, inward shift, `int()` truncation).
* The window is sliced inclusively (`img[top:bottom + 1, left:right + 1]`) as in
  `CropImage.crop`.
* `resize_linear` follows the documented `cv2.resize(..., INTER_LINEAR)` convention:
  half-pixel centre mapping `src = (dst + 0.5) * (in / out) - 0.5`, clamped to the source
  window (replicated border), bilinear weights, round to nearest.

The synthetic frame is non-biometric and defined by a formula that the Rust test reproduces.
Run from the repository root:  python3 crates/vision/tests/fixtures/pad_crop/generate_golden.py
"""

import math
import os

import numpy as np

FRAME_W = 160
FRAME_H = 120
OUT = 80

# (file name, face box (x1, y1, x2, y2) in continuous coordinates, scale)
CASES = [
    ("golden_scale_2_7.rgb", (60.0, 38.0, 96.5, 81.0), 2.7),
    ("golden_scale_4_0_edge.rgb", (118.0, 70.0, 142.0, 101.0), 4.0),
    ("golden_scale_2_7_upscale.rgb", (20.25, 20.0, 30.5, 32.0), 2.7),
]


def synthetic_frame():
    img = np.zeros((FRAME_H, FRAME_W, 3), dtype=np.uint8)
    for y in range(FRAME_H):
        for x in range(FRAME_W):
            img[y, x, 0] = (x * 7 + y * 3) & 0xFF
            img[y, x, 1] = ((x ^ y) * 5) & 0xFF
            img[y, x, 2] = (x * y) & 0xFF
    return img


def get_new_box(src_w, src_h, bbox, scale):
    x = bbox[0]
    y = bbox[1]
    box_w = bbox[2]
    box_h = bbox[3]

    scale = min((src_h - 1) / box_h, min((src_w - 1) / box_w, scale))

    new_width = box_w * scale
    new_height = box_h * scale
    center_x, center_y = box_w / 2 + x, box_h / 2 + y

    left_top_x = center_x - new_width / 2
    left_top_y = center_y - new_height / 2
    right_bottom_x = center_x + new_width / 2
    right_bottom_y = center_y + new_height / 2

    if left_top_x < 0:
        right_bottom_x -= left_top_x
        left_top_x = 0

    if left_top_y < 0:
        right_bottom_y -= left_top_y
        left_top_y = 0

    if right_bottom_x > src_w - 1:
        left_top_x -= right_bottom_x - src_w + 1
        right_bottom_x = src_w - 1

    if right_bottom_y > src_h - 1:
        left_top_y -= right_bottom_y - src_h + 1
        right_bottom_y = src_h - 1

    return int(left_top_x), int(left_top_y), int(right_bottom_x), int(right_bottom_y)


def resize_linear(src, out_w, out_h):
    in_h, in_w, _ = src.shape
    sx = in_w / out_w
    sy = in_h / out_h
    dst = np.zeros((out_h, out_w, 3), dtype=np.uint8)
    for v in range(out_h):
        fy = min(max((v + 0.5) * sy - 0.5, 0.0), in_h - 1)
        y0 = int(math.floor(fy))
        y1 = min(y0 + 1, in_h - 1)
        dy = fy - y0
        for u in range(out_w):
            fx = min(max((u + 0.5) * sx - 0.5, 0.0), in_w - 1)
            x0 = int(math.floor(fx))
            x1 = min(x0 + 1, in_w - 1)
            dx = fx - x0
            val = (
                (1 - dx) * (1 - dy) * src[y0, x0].astype(np.float64)
                + dx * (1 - dy) * src[y0, x1].astype(np.float64)
                + (1 - dx) * dy * src[y1, x0].astype(np.float64)
                + dx * dy * src[y1, x1].astype(np.float64)
            )
            dst[v, u] = np.clip(np.floor(val + 0.5), 0, 255).astype(np.uint8)
    return dst


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    img = synthetic_frame()
    for name, (x1, y1, x2, y2), scale in CASES:
        bbox = (x1, y1, x2 - x1, y2 - y1)
        left, top, right, bottom = get_new_box(FRAME_W, FRAME_H, bbox, scale)
        window = img[top : bottom + 1, left : right + 1]
        crop = resize_linear(window, OUT, OUT)
        with open(os.path.join(here, name), "wb") as fh:
            fh.write(crop.tobytes())
        print(f"{name}: window x={left} y={top} w={right - left + 1} h={bottom - top + 1}")


if __name__ == "__main__":
    main()
