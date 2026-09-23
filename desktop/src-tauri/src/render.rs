use serde::Deserialize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::ffmpeg::run;
use crate::ltc;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderOptions {
    pub input_path: String,
    pub output_path: String,

    /// FRAME_RATES のid。LTCの生成条件はここから引く
    pub fps_id: String,
    pub ltc_level_dbfs: f64,

    pub width: u32,
    pub height: u32,
    pub pixel_format: String,
    /// 映像のコーデック名 (ffprobeの codec_name)
    pub video_codec: String,
    /// フレームレートを有理数のまま受け取る (29.97を30と混同しないため)
    pub fps_num: u32,
    pub fps_den: u32,

    pub sample_rate: u32,
    pub bit_depth: u16,
    /// 元素材の音声チャンネル数。0 なら音声トラック無し
    pub source_channels: u32,

    pub preroll_frames: u32,
    pub postroll_frames: u32,
    pub total_samples: u64,
    /// 出力に埋め込むタイムコード (ドロップフレームは ';' 区切り)
    pub start_timecode: String,
}

/// 黒フレームは本編と同じコーデックで作らないと、ストリームコピーのまま連結できない。
fn black_encoder(video_codec: &str) -> Result<Vec<String>, String> {
    let args = match video_codec {
        "h264" => vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", "18"],
        "hevc" => vec!["-c:v", "libx265", "-preset", "veryfast", "-crf", "20"],
        "prores" => vec!["-c:v", "prores_ks", "-profile:v", "3"],
        other => {
            return Err(format!(
                "コーデック {other} ではプリロール/ポストロールを付けられません。\
                 黒フレームを本編と同じ形式で作れないためです。\
                 プリロールとポストロールを0にすれば、このファイルでも処理できます。"
            ))
        }
    };
    Ok(args.into_iter().map(String::from).collect())
}

fn audio_codec(bit_depth: u16) -> &'static str {
    if bit_depth == 24 {
        "pcm_s24le"
    } else {
        "pcm_s16le"
    }
}

/// 黒だけをエンコードしてMPEG-TSで書き出す。
/// TSにするのはSPS/PPSがフレーム内に入り、連結しても各セグメントが正しく復号できるため。
fn encode_black_segment(
    options: &RenderOptions,
    frames: u32,
    out: &Path,
) -> Result<(), String> {
    let duration = frames as f64 * options.fps_den as f64 / options.fps_num as f64;
    let size = format!("{}x{}", options.width, options.height);
    let rate = format!("{}/{}", options.fps_num, options.fps_den);
    let source = format!("color=c=black:s={size}:r={rate}:d={duration}");
    let out_str = out.to_string_lossy().to_string();

    let mut args: Vec<String> = vec![
        "-y", "-v", "error", "-f", "lavfi", "-i",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    args.push(source);
    args.extend(black_encoder(&options.video_codec)?);
    args.extend(
        ["-pix_fmt", &options.pixel_format, "-bsf:v", "h264_mp4toannexb", "-f", "mpegts"]
            .iter()
            .map(|s| s.to_string()),
    );
    args.push(out_str);

    // H.264以外は annexb のビットストリームフィルタが不要なので落とす
    if options.video_codec != "h264" {
        if let Some(pos) = args.iter().position(|a| a == "-bsf:v") {
            args.drain(pos..pos + 2);
        }
    }

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let output = run("ffmpeg", &arg_refs)?;
    if !output.status.success() {
        return Err(format!(
            "黒フレームの生成に失敗しました: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// 本編の映像を再エンコードせずTS化する
fn remux_main_to_ts(input: &str, out: &Path) -> Result<(), String> {
    let out_str = out.to_string_lossy().to_string();
    let output = run(
        "ffmpeg",
        &[
            "-y", "-v", "error", "-i", input, "-map", "0:v:0", "-c:v", "copy",
            "-bsf:v", "h264_mp4toannexb", "-f", "mpegts", &out_str,
        ],
    )?;
    if !output.status.success() {
        // H.264以外では annexb フィルタが使えないので、外して再試行する
        let retry = run(
            "ffmpeg",
            &[
                "-y", "-v", "error", "-i", input, "-map", "0:v:0", "-c:v", "copy",
                "-f", "mpegts", &out_str,
            ],
        )?;
        if !retry.status.success() {
            return Err(format!(
                "映像の読み出しに失敗しました: {}",
                String::from_utf8_lossy(&retry.stderr).trim()
            ));
        }
    }
    Ok(())
}

/// 元音声をモノラル化し、前後に無音を足してRchに載せるフィルタを組み立てる。
/// 音声が無い素材では無音を生成する。
fn build_audio_filter(options: &RenderOptions) -> String {
    let preroll_samples = options.preroll_frames as u64 * options.sample_rate as u64
        * options.fps_den as u64
        / options.fps_num as u64;

    if options.source_channels == 0 {
        return format!(
            "anullsrc=r={}:cl=mono,atrim=end_sample={}[r];[1:a][r]join=inputs=2:channel_layout=stereo[a]",
            options.sample_rate, options.total_samples
        );
    }

    // Web版と同じく L+R を平均してモノラル化する
    let downmix = if options.source_channels >= 2 {
        "pan=mono|c0=0.5*c0+0.5*c1".to_string()
    } else {
        "pan=mono|c0=c0".to_string()
    };

    let mut chain = format!("[2:a]aresample={},{}", options.sample_rate, downmix);
    if preroll_samples > 0 {
        chain.push_str(&format!(",adelay={preroll_samples}S:all=1"));
    }
    chain.push_str(&format!(
        ",apad=whole_len={total},atrim=end_sample={total}[r]",
        total = options.total_samples
    ));
    chain.push_str(";[1:a][r]join=inputs=2:channel_layout=stereo[a]");
    chain
}

/// 処理ごとに固有の作業ディレクトリを作る。
/// プロセス単位で共有すると、連続して処理を走らせたときに
/// 先に終わった側が後続の作業ファイルごと消してしまう。
fn unique_work_dir() -> Result<PathBuf, String> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);

    let dir = std::env::temp_dir().join(format!(
        "timecodemix-{}-{}-{}",
        std::process::id(),
        nanos,
        serial
    ));
    fs::create_dir_all(&dir).map_err(|e| format!("作業ディレクトリを作成できません: {e}"))?;
    Ok(dir)
}

pub fn render(options: RenderOptions) -> Result<String, String> {
    let work_dir = unique_work_dir()?;
    let result = render_inner(&options, &work_dir);
    let _ = fs::remove_dir_all(&work_dir);
    result
}

/// LTC波形を作業ディレクトリに書き出す。
///
/// 尺は total_samples から逆算するため、映像側の長さと必ず一致する。
fn write_ltc_file(
    options: &RenderOptions,
    config: &ltc::FrameRateConfig,
    start_frames: i64,
    work_dir: &Path,
) -> Result<PathBuf, String> {
    let path = work_dir.join("ltc.f32");
    let file = fs::File::create(&path)
        .map_err(|e| format!("タイムコードの書き出し先を作成できません: {e}"))?;
    let mut writer = io::BufWriter::new(file);

    let duration_seconds = options.total_samples as f64 / options.sample_rate as f64;
    ltc::write_ltc(
        &mut writer,
        duration_seconds,
        options.sample_rate,
        start_frames,
        options.ltc_level_dbfs,
        config,
    )
    .map_err(|e| format!("タイムコードの生成に失敗しました: {e}"))?;

    writer
        .flush()
        .map_err(|e| format!("タイムコードの書き出しに失敗しました: {e}"))?;
    Ok(path)
}

fn render_inner(options: &RenderOptions, work_dir: &Path) -> Result<String, String> {
    let config = ltc::find_frame_rate(&options.fps_id)
        .ok_or_else(|| format!("未知のフレームレートです: {}", options.fps_id))?;
    let start_frames = ltc::parse_timecode(&options.start_timecode, config)?;

    // tmcdに書く文字列はここで正規化する。
    // 受け取った文字列をそのまま使うと、"1:0:0:0" のような入力や
    // ドロップフレームの区切り文字の違いがそのまま出力に出てしまう。
    let canonical_timecode = ltc::format_timecode(
        &ltc::frame_to_timecode(start_frames, config),
        config.is_drop_frame,
    );

    // 0. LTC波形を生成する
    let ltc_path = write_ltc_file(options, config, start_frames, work_dir)?;

    // 1. 本編をストリームコピーでTS化
    let main_ts = work_dir.join("main.ts");
    remux_main_to_ts(&options.input_path, &main_ts)?;

    // 2. 必要なら黒セグメントを作る (黒だけのエンコードなので一瞬で終わる)
    let mut segments: Vec<PathBuf> = Vec::new();
    if options.preroll_frames > 0 {
        let pre = work_dir.join("pre.ts");
        encode_black_segment(options, options.preroll_frames, &pre)?;
        segments.push(pre);
    }
    segments.push(main_ts);
    if options.postroll_frames > 0 {
        let post = work_dir.join("post.ts");
        encode_black_segment(options, options.postroll_frames, &post)?;
        segments.push(post);
    }

    let concat_input = format!(
        "concat:{}",
        segments
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("|")
    );

    // 3. 連結した映像 + LTC + 元音声 を MOV に多重化する
    let sample_rate = options.sample_rate.to_string();
    let filter = build_audio_filter(options);
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-i".into(),
        concat_input,
        // LTCはヘッダの無い生データなので形式を明示する
        "-f".into(),
        "f32le".into(),
        "-ar".into(),
        sample_rate.clone(),
        "-ac".into(),
        "1".into(),
        "-i".into(),
        ltc_path.to_string_lossy().to_string(),
    ];

    if options.source_channels > 0 {
        args.extend(["-i".into(), options.input_path.clone()]);
    }

    args.extend([
        "-filter_complex".into(),
        filter,
        "-map".into(),
        "0:v".into(),
        "-map".into(),
        "[a]".into(),
        "-c:v".into(),
        "copy".into(),
        "-c:a".into(),
        audio_codec(options.bit_depth).into(),
        "-timecode".into(),
        canonical_timecode,
        "-f".into(),
        "mov".into(),
        options.output_path.clone(),
    ]);

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let output = run("ffmpeg", &arg_refs)?;
    if !output.status.success() {
        return Err(format!(
            "書き出しに失敗しました: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(options.output_path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS_NUM: u32 = 30000;
    const FPS_DEN: u32 = 1001;
    const SAMPLE_RATE: u32 = 48000;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    fn make_source(name: &str, seconds: u32, with_audio: bool) -> PathBuf {
        let path = temp_path(name);
        let path_str = path.to_string_lossy().to_string();
        let video = format!("testsrc2=size=320x240:rate=30000/1001:duration={seconds}");
        let audio = format!("sine=frequency=440:sample_rate=48000:duration={seconds}");

        let mut args = vec!["-y", "-v", "error", "-f", "lavfi", "-i", &video];
        if with_audio {
            args.extend(["-f", "lavfi", "-i", &audio]);
        }
        args.extend([
            "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
        ]);
        if with_audio {
            args.extend(["-c:a", "aac"]);
        }
        args.push(&path_str);

        let output = run("ffmpeg", &args).expect("ffmpegを実行できません");
        assert!(output.status.success(), "テスト素材の生成に失敗しました");
        path
    }

    fn probe_json(path: &Path) -> String {
        crate::ffmpeg::probe(&path.to_string_lossy()).expect("probeに失敗しました")
    }

    fn options_for(source: &Path, out: &Path, pre: u32, post: u32, source_channels: u32, total_samples: u64) -> RenderOptions {
        RenderOptions {
            input_path: source.to_string_lossy().to_string(),
            output_path: out.to_string_lossy().to_string(),
            fps_id: "29.97-ndf".into(),
            ltc_level_dbfs: -6.0,
            width: 320,
            height: 240,
            pixel_format: "yuv420p".into(),
            video_codec: "h264".into(),
            fps_num: FPS_NUM,
            fps_den: FPS_DEN,
            sample_rate: SAMPLE_RATE,
            bit_depth: 24,
            source_channels,
            preroll_frames: pre,
            postroll_frames: post,
            total_samples,
            start_timecode: "01:00:00:00".into(),
        }
    }

    fn total_samples_for(frames: u32) -> u64 {
        frames as u64 * SAMPLE_RATE as u64 * FPS_DEN as u64 / FPS_NUM as u64
    }

    #[test]
    fn renders_with_black_padding() {
        let source = make_source("tcmix_render_src.mp4", 2, true);
        // 2秒 = 約60フレーム。前に30フレーム、後ろに15フレーム足す
        let total_frames = 60 + 30 + 15;
        let total_samples = total_samples_for(total_frames);
        let out = temp_path("tcmix_render_out.mov");

        let options = options_for(&source, &out, 30, 15, 1, total_samples);
        render(options).expect("レンダリングに失敗しました");

        let json = probe_json(&out);
        assert!(json.contains("\"codec_name\": \"h264\""), "{json}");
        assert!(json.contains("\"codec_name\": \"pcm_s24le\""), "{json}");
        assert!(json.contains("\"channels\": 2"), "{json}");
        assert!(json.contains("01:00:00:00"), "タイムコードトラックがありません: {json}");

        // 全フレームを復号して、連結部分が壊れていないことを確かめる
        let decode = run("ffmpeg", &["-v", "error", "-i", &out.to_string_lossy(), "-f", "null", "-"])
            .expect("ffmpegを実行できません");
        let stderr = String::from_utf8_lossy(&decode.stderr);
        assert!(stderr.trim().is_empty(), "復号エラーが出ました: {stderr}");

        for f in [source, out] {
            let _ = fs::remove_file(f);
        }
    }

    #[test]
    fn preroll_section_is_black_and_content_survives() {
        let source = make_source("tcmix_black_src.mp4", 2, true);
        let total_frames = 60 + 30;
        let total_samples = total_samples_for(total_frames);
        let out = temp_path("tcmix_black_out.mov");

        render(options_for(&source, &out, 30, 0, 1, total_samples))
            .expect("レンダリングに失敗しました");

        // プリロール中(0.5秒)は真っ黒、本編(1.5秒)は黒くないはず。
        // ffmpegのログ出力に頼らず、1フレームをグレースケールの生データで取り出して平均する。
        let brightness = |at: &str| -> f64 {
            let output = run(
                "ffmpeg",
                &["-v", "error", "-ss", at, "-i", &out.to_string_lossy(), "-frames:v", "1",
                  "-pix_fmt", "gray", "-f", "rawvideo", "-"],
            )
            .expect("ffmpegを実行できません");
            assert!(!output.stdout.is_empty(), "フレームを取り出せませんでした");
            let sum: u64 = output.stdout.iter().map(|&b| b as u64).sum();
            sum as f64 / output.stdout.len() as f64
        };

        let dark = brightness("0.5");
        let content = brightness("1.5");
        assert!(dark >= 0.0 && dark < 20.0, "プリロールが黒くありません (YAVG={dark})");
        assert!(content > 30.0, "本編の映像が失われています (YAVG={content})");

        for f in [source, out] {
            let _ = fs::remove_file(f);
        }
    }

    #[test]
    fn renders_without_padding_using_stream_copy() {
        let source = make_source("tcmix_nopad_src.mp4", 2, true);
        let total_frames = 60;
        let total_samples = total_samples_for(total_frames);
        let out = temp_path("tcmix_nopad_out.mov");

        render(options_for(&source, &out, 0, 0, 1, total_samples))
            .expect("レンダリングに失敗しました");
        assert!(out.exists());

        for f in [source, out] {
            let _ = fs::remove_file(f);
        }
    }

    #[test]
    fn renders_source_without_audio_track() {
        let source = make_source("tcmix_noaudio_src.mp4", 2, false);
        let total_frames = 60 + 30;
        let total_samples = total_samples_for(total_frames);
        let out = temp_path("tcmix_noaudio_out.mov");

        render(options_for(&source, &out, 30, 0, 0, total_samples))
            .expect("音声トラックが無い素材でレンダリングに失敗しました");

        let json = probe_json(&out);
        assert!(json.contains("\"channels\": 2"), "{json}");

        for f in [source, out] {
            let _ = fs::remove_file(f);
        }
    }

    #[test]
    fn rejects_padding_for_unsupported_codec() {
        let result = black_encoder("vp9");
        assert!(result.is_err());
        let message = result.unwrap_err();
        assert!(message.contains("0にすれば"), "対処方法が案内されていません: {message}");
    }
}
