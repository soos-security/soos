import re

with open("crates/daemon/src/main.rs", "r") as f:
    content = f.read()

content = content.replace(
    "use soos_inference_ort::{",
    "use soos_inference_ort::{FaceDetector, LandmarkDetector, PadDetector, EmbeddingExtractor, "
)

content = content.replace(
    "let detector = Arc::new(OrtFaceDetector::new(registry.face_detector.path)?);",
    "let detector = Arc::new(OrtFaceDetector::new(registry.face_detector.path)?) as Arc<dyn FaceDetector>;"
)
content = content.replace(
    "let landmarks = Arc::new(OrtLandmarkDetector::new(registry.landmark_detector.path)?);",
    "let landmarks = Arc::new(OrtLandmarkDetector::new(registry.landmark_detector.path)?) as Arc<dyn LandmarkDetector>;"
)
content = content.replace(
    "let pad = Arc::new(OrtPadDetector::new(registry.pad_detector.path)?);",
    "let pad = Arc::new(OrtPadDetector::new(registry.pad_detector.path)?) as Arc<dyn PadDetector>;"
)
content = content.replace(
    "let extractor = Arc::new(OrtEmbeddingExtractor::new(registry.embedding_extractor.path)?);",
    "let extractor = Arc::new(OrtEmbeddingExtractor::new(registry.embedding_extractor.path)?) as Arc<dyn EmbeddingExtractor>;"
)

content = content.replace(
    "BioMasterKey::load_or_generate",
    "BioMasterKey::load_or_create"
)

content = content.replace(
    "EvMasterKey::load_or_generate",
    "EvMasterKey::load_or_create"
)

content = content.replace(
    "AuthorizationEngine::new(\n        config.thresholds.clone(),\n        config.rate_limit.clone(),\n    )",
    "AuthorizationEngine::with_rate_limiter(\n        config.thresholds.clone(),\n        soos_policy::RateLimiter::new(config.rate_limit.clone()),\n    )"
)


with open("crates/daemon/src/main.rs", "w") as f:
    f.write(content)
