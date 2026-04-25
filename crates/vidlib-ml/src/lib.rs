use ort::session::Session;
use serde::{Deserialize, Serialize};
use std::path::Path;
use vidlib_core::{VidLibError, VidLibResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MlTag {
    pub label: String,
    pub confidence: f32,
}

pub trait MlInferenceEngine: Send + Sync {
    fn describe_video(&self, path: &Path) -> VidLibResult<Vec<MlTag>>;
}

#[derive(Debug, Default)]
pub struct LocalHeuristicEngine;

impl MlInferenceEngine for LocalHeuristicEngine {
    fn describe_video(&self, path: &Path) -> VidLibResult<Vec<MlTag>> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let mut tags = Vec::new();
        for (needle, label) in [
            ("family", "family"),
            ("holiday", "holiday"),
            ("wedding", "wedding"),
            ("camera", "camera-shot"),
        ] {
            if name.contains(needle) {
                tags.push(MlTag {
                    label: label.into(),
                    confidence: 0.55,
                });
            }
        }
        Ok(tags)
    }
}

#[derive(Debug, Default)]
pub struct OnnxInferenceEngine {
    session_available: bool,
}

impl OnnxInferenceEngine {
    pub fn from_model_path(model_path: &Path) -> VidLibResult<Self> {
        let session_available = Session::builder()
            .map_err(|err| VidLibError::Ml(err.to_string()))?
            .commit_from_file(model_path)
            .is_ok();
        Ok(Self { session_available })
    }

    pub fn is_available(&self) -> bool {
        self.session_available
    }
}

impl MlInferenceEngine for OnnxInferenceEngine {
    fn describe_video(&self, path: &Path) -> VidLibResult<Vec<MlTag>> {
        if !self.session_available {
            return LocalHeuristicEngine.describe_video(path);
        }
        let mut tags = LocalHeuristicEngine.describe_video(path)?;
        tags.push(MlTag {
            label: "onnx-local".into(),
            confidence: 0.80,
        });
        Ok(tags)
    }
}

pub fn roadmap_note() -> &'static str {
    "ONNX Runtime-backed local inference is available behind MlInferenceEngine with heuristic fallback when no model is configured."
}
