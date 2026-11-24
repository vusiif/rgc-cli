#![forbid(unsafe_code)]

//! Comprehensive performance and latency benchmarking diagnostic tool.
//!
//! Evaluates end-to-end screen capture overhead:
//! 1. Native display/window capture latency
//! 2. Unified coordinate transform and image pipeline processing latency
//! 3. Set-of-Mark (SoM) edge gradient clustering & accessibility fusion latency
//! 4. PNG / JPEG / WebP image encoder throughput and compressed byte size

use std::time::Instant;

use image::DynamicImage;
use serde::{Deserialize, Serialize};

use crate::capture;
use crate::output::ImageFormat;
use crate::pipeline::{self, PipelineOptions};
use crate::som;

/// Statistical summary of latency samples.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencyStats {
    pub min_ms: f64,
    pub max_ms: f64,
    pub avg_ms: f64,
    pub p95_ms: f64,
}

impl LatencyStats {
    pub fn from_samples(mut samples: Vec<f64>) -> Self {
        if samples.is_empty() {
            return Self {
                min_ms: 0.0,
                max_ms: 0.0,
                avg_ms: 0.0,
                p95_ms: 0.0,
            };
        }

        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let min_ms = samples[0];
        let max_ms = samples[samples.len() - 1];
        let sum: f64 = samples.iter().sum();
        let avg_ms = sum / samples.len() as f64;

        let p95_index = ((samples.len() as f64) * 0.95).ceil() as usize - 1;
        let p95_ms = samples[p95_index.min(samples.len() - 1)];

        Self {
            min_ms,
            max_ms,
            avg_ms,
            p95_ms,
        }
    }
}

/// Benchmark metrics for a specific image encoder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderBenchmark {
    pub format: String,
    pub encode_ms: LatencyStats,
    pub output_bytes: u64,
    pub throughput_mpx_per_sec: f64,
}

/// Comprehensive benchmark report payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkReport {
    pub runs: u32,
    pub image_width: u32,
    pub image_height: u32,
    pub capture_ms: LatencyStats,
    pub pipeline_ms: LatencyStats,
    pub som_ms: LatencyStats,
    pub encoders: Vec<EncoderBenchmark>,
}

/// Run end-to-end performance benchmark and print structured results.
pub fn run_benchmark(runs: u32, is_json: bool) -> Result<(), String> {
    let runs = runs.clamp(1, 100);

    // 1. Warm-up capture or fallback to synthetic 1920x1080 canvas if running headless
    let (sample_img, is_synthetic) =
        match capture::capture_screen_with_details(None, None, crate::cli::CoordMode::Physical) {
            Ok(c) => (c.image, false),
            Err(_) => {
                let mut img = DynamicImage::new_rgba8(1920, 1080);
                let rgba = img.as_mut_rgba8().unwrap();
                for (x, y, pixel) in rgba.enumerate_pixels_mut() {
                    let r = ((x * 255) / 1920) as u8;
                    let g = ((y * 255) / 1080) as u8;
                    let b = ((x + y) % 256) as u8;
                    *pixel = image::Rgba([r, g, b, 255]);
                }
                (img, true)
            }
        };

    let width = sample_img.width();
    let height = sample_img.height();
    let total_pixels = (width as f64) * (height as f64);
    let mega_pixels = total_pixels / 1_000_000.0;

    // 2. Measure capture latency (if native display available)
    let mut capture_samples = Vec::with_capacity(runs as usize);
    if !is_synthetic {
        for _ in 0..runs {
            let t0 = Instant::now();
            let _ =
                capture::capture_screen_with_details(None, None, crate::cli::CoordMode::Physical);
            let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
            capture_samples.push(elapsed_ms);
        }
    } else {
        capture_samples.push(0.0);
    }
    let capture_stats = LatencyStats::from_samples(capture_samples);

    // 3. Measure unified pipeline latency (scaling, overlays, coordinates)
    let mut pipeline_samples = Vec::with_capacity(runs as usize);
    let pipeline_opts = PipelineOptions {
        grid: Some(100),
        timestamp: true,
        ..Default::default()
    };
    for _ in 0..runs {
        let t0 = Instant::now();
        let _ = pipeline::execute_image_pipeline(
            sample_img.clone(),
            1.0,
            (0, 0),
            Some((0, 0)),
            &pipeline_opts,
        );
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        pipeline_samples.push(elapsed_ms);
    }
    let pipeline_stats = LatencyStats::from_samples(pipeline_samples);

    // 4. Measure Set-of-Mark (SoM) edge detection and clustering latency
    let mut som_samples = Vec::with_capacity(runs as usize);
    for _ in 0..runs {
        let t0 = Instant::now();
        let _ = som::detect_ui_elements(&sample_img, 25, 50);
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        som_samples.push(elapsed_ms);
    }
    let som_stats = LatencyStats::from_samples(som_samples);

    // 5. Measure Encoder throughput and size (PNG, JPEG, WebP)
    let mut encoder_benchmarks = Vec::new();
    let test_formats = [
        (ImageFormat::Png, None),
        (ImageFormat::Jpeg, Some(85u8)),
        (ImageFormat::Webp, None),
    ];

    for (fmt, quality) in test_formats {
        let mut encode_samples = Vec::with_capacity(runs as usize);
        let mut out_len = 0u64;

        for _ in 0..runs {
            let t0 = Instant::now();
            let encoded = crate::output::encode_image_bytes(&sample_img, fmt, quality);
            let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
            encode_samples.push(elapsed_ms);
            if let Ok(bytes) = encoded {
                out_len = bytes.len() as u64;
            }
        }

        let stats = LatencyStats::from_samples(encode_samples);
        let avg_sec = (stats.avg_ms / 1000.0).max(0.0001);
        let throughput = mega_pixels / avg_sec;

        encoder_benchmarks.push(EncoderBenchmark {
            format: fmt.to_string(),
            encode_ms: stats,
            output_bytes: out_len,
            throughput_mpx_per_sec: (throughput * 100.0).round() / 100.0,
        });
    }

    let report = BenchmarkReport {
        runs,
        image_width: width,
        image_height: height,
        capture_ms: capture_stats,
        pipeline_ms: pipeline_stats,
        som_ms: som_stats,
        encoders: encoder_benchmarks,
    };

    if is_json {
        let json_str = serde_json::to_string_pretty(&report)
            .map_err(|e| format!("Failed to serialize benchmark report: {e}"))?;
        println!("{}", json_str);
    } else {
        print_human_benchmark_report(&report, is_synthetic);
    }

    Ok(())
}

fn print_human_benchmark_report(report: &BenchmarkReport, is_synthetic: bool) {
    println!("============================================================");
    println!("             rgc-cli Performance Benchmark Diagnostic        ");
    println!("============================================================");
    println!(
        "Target Canvas : {}x{} ({:.2} MPx) [{}]",
        report.image_width,
        report.image_height,
        (report.image_width as f64 * report.image_height as f64) / 1_000_000.0,
        if is_synthetic {
            "Synthetic Fallback"
        } else {
            "Physical Screen"
        }
    );
    println!("Test Iterations: {}", report.runs);
    println!("------------------------------------------------------------");
    println!("STAGE                   MIN (ms)   AVG (ms)   P95 (ms)   MAX (ms)");
    println!("------------------------------------------------------------");

    println!(
        "{:<20} {:>9.2}  {:>9.2}  {:>9.2}  {:>9.2}",
        "Screen Capture",
        report.capture_ms.min_ms,
        report.capture_ms.avg_ms,
        report.capture_ms.p95_ms,
        report.capture_ms.max_ms
    );

    println!(
        "{:<20} {:>9.2}  {:>9.2}  {:>9.2}  {:>9.2}",
        "Pipeline (Grid+TS)",
        report.pipeline_ms.min_ms,
        report.pipeline_ms.avg_ms,
        report.pipeline_ms.p95_ms,
        report.pipeline_ms.max_ms
    );

    println!(
        "{:<20} {:>9.2}  {:>9.2}  {:>9.2}  {:>9.2}",
        "SoM UI Clustering",
        report.som_ms.min_ms,
        report.som_ms.avg_ms,
        report.som_ms.p95_ms,
        report.som_ms.max_ms
    );

    println!("------------------------------------------------------------");
    println!("ENCODER      AVG (ms)   SIZE (KB)   THROUGHPUT (MPx/s)");
    println!("------------------------------------------------------------");
    for enc in &report.encoders {
        let size_kb = enc.output_bytes as f64 / 1024.0;
        println!(
            "{:<10} {:>10.2}  {:>10.1}  {:>18.2}",
            enc.format.to_uppercase(),
            enc.encode_ms.avg_ms,
            size_kb,
            enc.throughput_mpx_per_sec
        );
    }
    println!("============================================================");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latency_stats_calculation() {
        let samples = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        let stats = LatencyStats::from_samples(samples);
        assert_eq!(stats.min_ms, 10.0);
        assert_eq!(stats.max_ms, 50.0);
        assert_eq!(stats.avg_ms, 30.0);
        assert_eq!(stats.p95_ms, 50.0);

        let empty_stats = LatencyStats::from_samples(vec![]);
        assert_eq!(empty_stats.avg_ms, 0.0);
    }

    #[test]
    fn test_benchmark_report_serialization() {
        let report = BenchmarkReport {
            runs: 5,
            image_width: 1920,
            image_height: 1080,
            capture_ms: LatencyStats {
                min_ms: 12.0,
                max_ms: 18.0,
                avg_ms: 15.0,
                p95_ms: 17.5,
            },
            pipeline_ms: LatencyStats {
                min_ms: 2.0,
                max_ms: 4.0,
                avg_ms: 3.0,
                p95_ms: 3.8,
            },
            som_ms: LatencyStats {
                min_ms: 8.0,
                max_ms: 12.0,
                avg_ms: 9.5,
                p95_ms: 11.2,
            },
            encoders: vec![EncoderBenchmark {
                format: "png".to_string(),
                encode_ms: LatencyStats {
                    min_ms: 20.0,
                    max_ms: 25.0,
                    avg_ms: 22.0,
                    p95_ms: 24.5,
                },
                output_bytes: 524288,
                throughput_mpx_per_sec: 94.25,
            }],
        };

        let json = serde_json::to_string(&report).expect("Serialization should succeed");
        assert!(json.contains("\"imageWidth\":1920"));
        assert!(json.contains("\"throughputMpxPerSec\":94.25"));
    }
}
