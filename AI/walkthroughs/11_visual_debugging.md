# Walkthrough 11: Visual Debugging GUI

## 1. Context

Users experience issues enrolling their faces (e.g., lighting problems, PAD failure, bounding box issues). The `debug-vision` command is introduced in `soos-enroll` to help visualize what the AI models actually see and output.

## 2. Implementation

- **`html_report.rs`**: Implements a zero-dependency Base64 and BMP encoder.
- **`debug-vision` subcommand**: Added to `soos-enroll` to capture a frame from the `CameraManager`, run it through the `VisionPipeline` detectors, and generate a standalone HTML report.
- **HTML Report**: Embeds the frame as a Base64-encoded BMP and overlays the bounding boxes, confidence scores, and 5-point landmarks using HTML5 Canvas.

## 3. Usage

Run the enrollment CLI in debug mode:

```bash
sudo soos-enroll debug-vision
```

For mock testing without a camera:

```bash
sudo soos-enroll --mock debug-vision
```

The command will generate an HTML file at `/tmp/soos-debug.html`. Open this file in any web browser to see the live feed detection overlay.
