use serde::Deserialize;
use std::path::Path;
use std::process::Command;
use vidlib_core::{VidLibError, VidLibResult};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaMetadata {
    pub duration_seconds: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub codec: Option<String>,
}

pub trait MetadataProvider: Send + Sync {
    fn extract(&self, path: &Path) -> VidLibResult<MediaMetadata>;
}

pub fn ffprobe_available() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[derive(Debug, Clone, Default)]
pub struct FfprobeMetadataProvider;

impl MetadataProvider for FfprobeMetadataProvider {
    fn extract(&self, path: &Path) -> VidLibResult<MediaMetadata> {
        if !ffprobe_available() {
            return Ok(MediaMetadata::default());
        }
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,width,height:format=duration",
                "-of",
                "json",
            ])
            .arg(path)
            .output();

        match output {
            Ok(output) if output.status.success() => parse_ffprobe_output(&output.stdout),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let _ = stderr;
                Ok(MediaMetadata::default())
            }
            Err(_) => Ok(MediaMetadata::default()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProbeResult {
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

pub fn parse_ffprobe_output(bytes: &[u8]) -> VidLibResult<MediaMetadata> {
    let parsed: ProbeResult = serde_json::from_slice(bytes)
        .map_err(|err| VidLibError::Metadata(format!("parsing ffprobe json: {err}")))?;
    let stream = parsed.streams.into_iter().next();
    Ok(MediaMetadata {
        duration_seconds: parsed
            .format
            .and_then(|f| f.duration)
            .and_then(|d| d.parse::<f64>().ok()),
        width: stream.as_ref().and_then(|s| s.width),
        height: stream.as_ref().and_then(|s| s.height),
        codec: stream.and_then(|s| s.codec_name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffprobe_json() {
        let json = br#"{
          "streams": [{"codec_name":"h264","width":1280,"height":720}],
          "format": {"duration": "12.5"}
        }"#;
        let metadata = parse_ffprobe_output(json).unwrap();
        assert_eq!(metadata.codec.as_deref(), Some("h264"));
        assert_eq!(metadata.width, Some(1280));
        assert_eq!(metadata.height, Some(720));
        assert_eq!(metadata.duration_seconds, Some(12.5));
    }
}
