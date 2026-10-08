//! What ffprobe says a file holds, and from that, whether every browser plays it as it is or
//! what copy to make. Pure: the probe is the converter's job, the decision is tested here on
//! probes written out by hand.

use ccosel_proto::fs::Shown;
use serde::Deserialize;

use super::{Plan, Recipe};

/// The parts of `ffprobe -show_format -show_streams` the decision needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    /// ffprobe's demuxer names, comma-separated: `mov,mp4,m4a,3gp,3g2,mj2`, `matroska,webm`.
    pub format: String,
    pub duration_us: Option<u64>,
    /// The first video stream that is a picture in motion, not an album cover.
    pub video: Option<VideoStream>,
    pub audio: Option<AudioStream>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VideoStream {
    pub codec: String,
    pub profile: String,
    pub pix_fmt: String,
    /// The transfer function: `smpte2084` (PQ) and `arib-std-b67` (HLG) are HDR.
    pub transfer: String,
    /// `progressive`, or how the fields are ordered (`tt`, `bb`, …) when interlaced.
    pub field_order: String,
    pub width: u32,
    pub height: u32,
    /// Degrees, from the display matrix: a phone held upright records ±90.
    pub rotation: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioStream {
    pub codec: String,
    pub sample_rate: u32,
}

#[derive(Deserialize)]
struct Json {
    #[serde(default)]
    format: JsonFormat,
    #[serde(default)]
    streams: Vec<JsonStream>,
}

#[derive(Deserialize, Default)]
struct JsonFormat {
    #[serde(default)]
    format_name: String,
    duration: Option<String>,
}

#[derive(Deserialize, Default)]
struct JsonStream {
    #[serde(default)]
    codec_type: String,
    #[serde(default)]
    codec_name: String,
    #[serde(default)]
    profile: String,
    #[serde(default)]
    pix_fmt: String,
    #[serde(default)]
    color_transfer: String,
    #[serde(default)]
    field_order: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    sample_rate: Option<String>,
    #[serde(default)]
    disposition: JsonDisposition,
    #[serde(default)]
    side_data_list: Vec<JsonSideData>,
}

#[derive(Deserialize, Default)]
struct JsonDisposition {
    #[serde(default)]
    attached_pic: u8,
}

#[derive(Deserialize, Default)]
struct JsonSideData {
    rotation: Option<f64>,
}

impl Probe {
    /// What `ffprobe -of json -show_format -show_streams` printed, or `None` if it isn't that.
    pub fn from_json(json: &str) -> Option<Self> {
        let j: Json = serde_json::from_str(json).ok()?;
        let video = j
            .streams
            .iter()
            .find(|s| s.codec_type == "video" && s.disposition.attached_pic == 0)
            .map(|s| VideoStream {
                codec: s.codec_name.clone(),
                profile: s.profile.clone(),
                pix_fmt: s.pix_fmt.clone(),
                transfer: s.color_transfer.clone(),
                field_order: s.field_order.clone(),
                width: s.width,
                height: s.height,
                rotation: s
                    .side_data_list
                    .iter()
                    .find_map(|d| d.rotation)
                    .map_or(0, |r| r.round() as i32),
            });
        let audio = j
            .streams
            .iter()
            .find(|s| s.codec_type == "audio")
            .map(|s| AudioStream {
                codec: s.codec_name.clone(),
                sample_rate: s
                    .sample_rate
                    .as_deref()
                    .and_then(|r| r.parse().ok())
                    .unwrap_or(0),
            });
        let duration_us = j
            .format
            .duration
            .as_deref()
            .and_then(|d| d.parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d > 0.0)
            .map(|d| (d * 1_000_000.0) as u64);
        Some(Self {
            format: j.format.format_name,
            duration_us,
            video,
            audio,
        })
    }

    fn container(&self, name: &str) -> bool {
        self.format.split(',').any(|f| f == name)
    }
}

impl VideoStream {
    /// Whether this is H.264 every browser decodes: 8-bit 4:2:0 in a profile below High 10.
    pub fn is_web_h264(&self) -> bool {
        self.codec == "h264"
            && self.pix_fmt == "yuv420p"
            && matches!(
                self.profile.as_str(),
                "Baseline" | "Constrained Baseline" | "Main" | "High" | "Extended" | ""
            )
            && !self.is_hdr()
    }

    pub fn is_hdr(&self) -> bool {
        matches!(self.transfer.as_str(), "smpte2084" | "arib-std-b67")
    }

    pub fn is_interlaced(&self) -> bool {
        matches!(self.field_order.as_str(), "tt" | "bb" | "tb" | "bt")
    }

    /// Its size as it is meant to be seen, turned for its rotation.
    pub fn shown_size(&self) -> Option<(u32, u32)> {
        if self.width == 0 || self.height == 0 {
            return None;
        }
        Some(if self.rotation.rem_euclid(180) == 90 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        })
    }
}

/// Formats ffprobe reads that aren't pictures, video or sound, though it finds a "stream" in
/// them: text it would draw as ANSI art, and raw data.
const NOT_MEDIA: &[&str] = &["tty", "data", "bin", "subviewer", "srt", "webvtt", "ass"];

/// How a file ffprobe made `probe` of, with extension `ext` (lowercase), is shown.
pub fn media(probe: &Probe, ext: &str) -> Plan {
    if NOT_MEDIA.iter().any(|f| probe.container(f)) {
        return Plan::NotMedia;
    }
    // A picture format only ffmpeg recognised: its image demuxers are `image2` and `*_pipe`.
    if probe.format == "image2" || probe.format.ends_with("_pipe") {
        // No size means ffmpeg only went by the name: text or noise called `.jpg`.
        return match &probe.video {
            Some(v) if v.codec != "ansi" && v.width > 0 && v.height > 0 => Plan::Convert {
                shown: Shown::Picture,
                recipe: Recipe::Picture {
                    magick: None,
                    magick_first: false,
                    orientation: 1,
                },
            },
            _ => Plan::NotMedia,
        };
    }
    if let Some(v) = &probe.video {
        if v.codec == "ansi" {
            return Plan::NotMedia;
        }
        return video(probe, v, ext);
    }
    if let Some(a) = &probe.audio {
        return audio(probe, a, ext);
    }
    Plan::NotMedia
}

fn video(probe: &Probe, v: &VideoStream, ext: &str) -> Plan {
    let audio = probe.audio.as_ref().map(|a| a.codec.as_str());
    let plays_as_is = if probe.container("mp4") && matches!(ext, "mp4" | "m4v") {
        v.is_web_h264() && matches!(audio, None | Some("aac" | "mp3"))
    } else if probe.container("webm") && ext == "webm" {
        // Matroska and WebM share a demuxer, so it's the codecs that say which this is.
        let codec_ok = match v.codec.as_str() {
            "vp8" => true,
            "vp9" | "av1" => v.pix_fmt == "yuv420p" && !v.is_hdr(),
            _ => false,
        };
        codec_ok && matches!(audio, None | Some("opus" | "vorbis"))
    } else {
        false
    };
    if plays_as_is {
        return Plan::AsIs {
            shown: Shown::Video,
            content_type: if ext == "webm" {
                "video/webm"
            } else {
                "video/mp4"
            },
        };
    }
    Plan::Convert {
        shown: Shown::Video,
        recipe: Recipe::Video {
            copy_video: v.is_web_h264() && !v.is_interlaced(),
            audio: audio.map(|a| a == "aac"),
            tone_map: v.is_hdr(),
            deinterlace: v.is_interlaced(),
            duration_us: probe.duration_us,
        },
    }
}

fn audio(probe: &Probe, a: &AudioStream, ext: &str) -> Plan {
    let codec = a.codec.as_str();
    let content_type = match codec {
        "mp3" if probe.container("mp3") && ext == "mp3" => Some("audio/mpeg"),
        "aac" if probe.container("aac") && ext == "aac" => Some("audio/aac"),
        "aac" if probe.container("mp4") && matches!(ext, "m4a" | "m4b" | "mp4") => {
            Some("audio/mp4")
        }
        "flac" if probe.container("flac") && ext == "flac" => Some("audio/flac"),
        "pcm_s16le" | "pcm_s24le" | "pcm_f32le" | "pcm_u8"
            if probe.container("wav") && ext == "wav" =>
        {
            Some("audio/wav")
        }
        "opus" | "vorbis" if probe.container("ogg") && matches!(ext, "ogg" | "oga" | "opus") => {
            Some("audio/ogg")
        }
        "opus" | "vorbis" if probe.container("webm") && matches!(ext, "webm" | "weba") => {
            Some("audio/webm")
        }
        _ => None,
    };
    match content_type {
        Some(content_type) => Plan::AsIs {
            shown: Shown::Audio,
            content_type,
        },
        None => Plan::Convert {
            shown: Shown::Audio,
            recipe: Recipe::Audio {
                // AAC goes no higher than 96 kHz, and nobody hears past 48.
                resample: a.sample_rate > 48_000,
            },
        },
    }
}
