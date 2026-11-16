//! Кодеки и кодировщики: CODECS, AUDIO_OUT, encoder_args, detect_gpu.
use crate::procs;
use std::time::Duration;

/// Видеокодек на выходе: ffmpeg vcodec, acodec, контейнер, доп. аргументы, суффикс в имени файла.
/// copy — всё None (без перекодирования).
pub struct Codec {
    pub vcodec: Option<&'static str>,
    pub acodec: Option<&'static str>,
    pub ext: Option<&'static str>,
    pub extra: &'static [&'static str],
    pub tag: Option<&'static str>,
}

pub fn codec(id: &str) -> Option<Codec> {
    Some(match id {
        "h264" => Codec { vcodec: Some("libx264"), acodec: Some("aac"), ext: Some("mp4"), extra: &["-pix_fmt", "yuv420p"], tag: Some("h264") },
        "h265" => Codec {
            vcodec: Some("libx265"), acodec: Some("aac"), ext: Some("mp4"),
            extra: &["-pix_fmt", "yuv420p", "-tag:v", "hvc1"], tag: Some("hevc"),
        },
        "prores" => Codec {
            vcodec: Some("prores_ks"), acodec: Some("pcm_s24le"), ext: Some("mov"),
            extra: &["-profile:v", "2", "-pix_fmt", "yuv422p10le"], tag: Some("prores"),
        },
        "copy" => Codec { vcodec: None, acodec: None, ext: None, extra: &[], tag: None },
        _ => return None,
    })
}

/// Только звук: id -> (ffmpeg-кодек, расширение).
pub fn audio_out(id: &str) -> Option<(&'static str, &'static str)> {
    match id {
        "mp3" => Some(("libmp3lame", "mp3")),
        "flac" => Some(("flac", "flac")),
        "m4a" => Some(("aac", "m4a")),
        _ => None,
    }
}

/// Больше 8 потоков x264 почти не ускоряют, а память растёт.
pub fn cpu_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8)
}

/// Кодировщик видеокарты, который реально работает.
#[derive(Debug, Clone, PartialEq)]
pub struct Gpu {
    pub vendor: &'static str,
    pub short: &'static str,
    pub h264: &'static str,
    pub h265: &'static str,
}

impl Gpu {
    pub fn enc(&self, codec: &str) -> &'static str {
        if codec == "h265" { self.h265 } else { self.h264 }
    }
}

pub const GPU_FAMILIES: &[Gpu] = &[
    Gpu { vendor: "NVIDIA", short: "NVENC", h264: "h264_nvenc", h265: "hevc_nvenc" },
    Gpu { vendor: "AMD", short: "AMF", h264: "h264_amf", h265: "hevc_amf" },
    Gpu { vendor: "Intel", short: "QSV", h264: "h264_qsv", h265: "hevc_qsv" },
];

/// Первый кодировщик видеокарты, который реально работает (пробное кодирование 0,2 с).
pub fn detect_gpu() -> Option<Gpu> {
    for g in GPU_FAMILIES {
        let args: Vec<String> = [
            "ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=black:s=640x360:d=0.2",
            "-c:v", g.h264, "-f", "null", "-",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if let Ok(out) = procs::run_quiet(&args, Duration::from_secs(20)) {
            if out.code == Some(0) {
                return Some(g.clone());
            }
        }
    }
    None
}

/// Аргументы видеокодировщика. gpu — None: процессор.
pub fn encoder_args(codec: &str, crf: i64, preset: &str, gpu: Option<&Gpu>) -> Vec<String> {
    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let crf_s = crf.to_string();
    if let Some(g) = gpu {
        let enc = g.enc(codec);
        if g.vendor == "NVIDIA" {
            let p = match preset {
                "ultrafast" | "superfast" => "p1",
                "veryfast" => "p2",
                "faster" | "fast" => "p3",
                "medium" | "slow" => "p4",
                "slower" => "p5",
                "veryslow" => "p6",
                _ => "p4",
            };
            return v(&["-c:v", enc, "-preset", p, "-tune", "hq", "-rc", "vbr", "-cq", &crf_s, "-b:v", "0"]);
        }
        if g.vendor == "AMD" {
            let q = match preset {
                "ultrafast" | "superfast" | "veryfast" | "faster" => "speed",
                "fast" | "medium" => "balanced",
                _ => "quality",
            };
            return v(&["-c:v", enc, "-quality", q, "-rc", "cqp", "-qp_i", &crf_s, "-qp_p", &(crf + 2).to_string()]);
        }
        let qp = match preset { "ultrafast" | "superfast" => "veryfast", p => p };
        return v(&["-c:v", enc, "-preset", qp, "-global_quality", &crf_s]);
    }
    let t = cpu_threads();
    if codec == "h265" {
        return v(&["-c:v", "libx265", "-preset", preset, "-crf", &crf_s, "-x265-params",
            &format!("pools={t}:rc-lookahead=20:log-level=error")]);
    }
    v(&["-c:v", "libx264", "-preset", preset, "-crf", &crf_s, "-threads", &t.to_string(), "-x264-params", "rc-lookahead=20"])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enc_args() {
        let nv = &GPU_FAMILIES[0];
        assert_eq!(encoder_args("h265", 18, "slow", Some(nv)).join(" "),
            "-c:v hevc_nvenc -preset p4 -tune hq -rc vbr -cq 18 -b:v 0");
        assert_eq!(encoder_args("h264", 18, "fast", Some(&GPU_FAMILIES[1])).join(" "),
            "-c:v h264_amf -quality balanced -rc cqp -qp_i 18 -qp_p 20");
        assert_eq!(encoder_args("h264", 20, "superfast", Some(&GPU_FAMILIES[2])).join(" "),
            "-c:v h264_qsv -preset veryfast -global_quality 20");
        let t = cpu_threads();
        assert_eq!(encoder_args("h264", 16, "slow", None).join(" "),
            format!("-c:v libx264 -preset slow -crf 16 -threads {t} -x264-params rc-lookahead=20"));
        assert_eq!(encoder_args("h265", 16, "slow", None).join(" "),
            format!("-c:v libx265 -preset slow -crf 16 -x265-params pools={t}:rc-lookahead=20:log-level=error"));
        assert!(codec("copy").unwrap().vcodec.is_none());
        assert_eq!(audio_out("flac"), Some(("flac", "flac")));
        assert!(codec("mp3").is_none());
    }
}
