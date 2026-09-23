use serde::Deserialize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::ffmpeg::run_with_progress;
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

/// セグメントの連結方法。コーデックによって使える手段が違う。
#[derive(Debug, Clone, Copy, PartialEq)]
enum ConcatStrategy {
    /// H.264 / HEVC 用。
    /// これらはパラメータセット(SPS/PPS等)をコンテナ側に1つだけ持つため、
    /// 別々にエンコードしたセグメントをそのまま繋ぐと後続が正しく復号できない。
    /// MPEG-TSはフレーム内にパラメータセットを埋め込むのでこの問題が起きない。
    MpegTs { annexb_filter: &'static str },
    /// ProRes のような全フレームがキーフレームの形式用。
    /// パラメータセットの問題が無いので、MOVのまま concat デマルチプレクサで繋げる。
    /// (ProResはMPEG-TSに入れられないので、そもそもTS経由は使えない)
    ConcatDemuxer,
}

#[derive(Debug)]
struct CodecProfile {
    strategy: ConcatStrategy,
    /// 黒フレームを本編と同じ形式で作るためのエンコーダ指定
    black_encoder: &'static [&'static str],
    segment_extension: &'static str,
    segment_format: &'static str,
}

fn codec_profile(video_codec: &str) -> Result<CodecProfile, String> {
    match video_codec {
        "h264" => Ok(CodecProfile {
            strategy: ConcatStrategy::MpegTs { annexb_filter: "h264_mp4toannexb" },
            black_encoder: &["-c:v", "libx264", "-preset", "veryfast", "-crf", "18"],
            segment_extension: "ts",
            segment_format: "mpegts",
        }),
        "hevc" => Ok(CodecProfile {
            strategy: ConcatStrategy::MpegTs { annexb_filter: "hevc_mp4toannexb" },
            black_encoder: &["-c:v", "libx265", "-preset", "veryfast", "-crf", "20"],
            segment_extension: "ts",
            segment_format: "mpegts",
        }),
        "prores" => Ok(CodecProfile {
            strategy: ConcatStrategy::ConcatDemuxer,
            black_encoder: &["-c:v", "prores_ks", "-profile:v", "3"],
            segment_extension: "mov",
            segment_format: "mov",
        }),
        other => Err(format!(
            "コーデック {other} ではプリロール/ポストロールを付けられません。\
             黒フレームを本編と同じ形式で作れないためです。\
             プリロールとポストロールを0にすれば、このファイルでも処理できます。"
        )),
    }
}

fn audio_codec(bit_depth: u16) -> &'static str {
    if bit_depth == 24 {
        "pcm_s24le"
    } else {
        "pcm_s16le"
    }
}

/// 黒だけを本編と同じ形式でエンコードする。
///
/// H.264なら一瞬で終わるが、ProResのようなイントラ圧縮では1080pで
/// 1フレームあたり1.5MB近くになり、数秒かかることがある。
/// 無反応にならないよう進捗を報告する。
fn encode_black_segment(
    options: &RenderOptions,
    profile: &CodecProfile,
    frames: u32,
    out: &Path,
    on_progress: impl FnMut(f64),
) -> Result<(), String> {
    let duration = frames as f64 * options.fps_den as f64 / options.fps_num as f64;
    let size = format!("{}x{}", options.width, options.height);
    let rate = format!("{}/{}", options.fps_num, options.fps_den);
    let source = format!("color=c=black:s={size}:r={rate}:d={duration}");
    let out_str = out.to_string_lossy().to_string();

    let mut args: Vec<String> = ["-y", "-v", "error", "-f", "lavfi", "-i"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    args.push(source);
    args.extend(profile.black_encoder.iter().map(|s| s.to_string()));
    args.extend(["-pix_fmt".to_string(), options.pixel_format.clone()]);

    if let ConcatStrategy::MpegTs { annexb_filter } = profile.strategy {
        args.extend(["-bsf:v".to_string(), annexb_filter.to_string()]);
    }

    args.extend(["-f".to_string(), profile.segment_format.to_string()]);
    args.push(out_str);

    let duration_us = (duration * 1_000_000.0) as u64;
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_with_progress(&arg_refs, duration_us, on_progress)
        .map_err(|e| format!("黒フレームの生成に失敗しました: {e}"))?;
    Ok(())
}

/// 本編の映像を再エンコードせずセグメント化する。
///
/// 再エンコードはしないが素材のサイズぶんI/Oが発生する。ProResのような
/// 大容量素材ではここが最も時間を食うため、進捗を報告する。
fn remux_main_segment(
    input: &str,
    profile: &CodecProfile,
    out: &Path,
    duration_us: u64,
    on_progress: impl FnMut(f64),
) -> Result<(), String> {
    let out_str = out.to_string_lossy().to_string();
    let mut args: Vec<String> = [
        "-y", "-v", "error", "-i", input, "-map", "0:v:0", "-c:v", "copy",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    if let ConcatStrategy::MpegTs { annexb_filter } = profile.strategy {
        args.extend(["-bsf:v".to_string(), annexb_filter.to_string()]);
    }
    args.extend(["-f".to_string(), profile.segment_format.to_string()]);
    args.push(out_str);

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_with_progress(&arg_refs, duration_us, on_progress)
        .map_err(|e| format!("映像の読み出しに失敗しました: {e}"))?;

    // ffmpegは対応していないコーデックをMPEG-TSへ入れようとしたとき、
    // 終了コード0のままデータストリームとして書き出してしまうことがある。
    // 壊れた出力をそのまま進めないよう、映像が入っているか確かめる。
    let probe_json = crate::ffmpeg::probe(&out.to_string_lossy())?;
    if !probe_json.contains("\"codec_type\": \"video\"") {
        return Err(format!(
            "この映像({})は中間形式に変換できませんでした。\
             プリロールとポストロールを0にすれば処理できる場合があります。",
            profile.segment_format
        ));
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

/// 進捗の通知。フロントエンドはこれを受けてバーと文言を更新する。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderProgress {
    /// 0.0 〜 1.0
    pub ratio: f64,
    pub message: String,
}

/// 工程ごとの重み。
///
/// 素材が大きいほど、本編を読み出すTS化と最終の多重化が支配的になる
/// (どちらも素材のサイズぶんI/Oが走る)。黒の生成とLTC生成は尺に対して
/// ほぼ一定時間で終わるので小さく取る。
/// 黒の生成は、H.264なら一瞬だがProResでは数秒かかる。
/// コーデック差を厳密に重みへ反映するのは難しいので、どちらでも極端に
/// 不自然にならない配分にし、各工程の中で進捗を出してバーを動かし続ける。
const WEIGHT_LTC: f64 = 0.05;
const WEIGHT_REMUX: f64 = 0.30;
const WEIGHT_BLACK: f64 = 0.15;
const WEIGHT_OUTPUT: f64 = 0.50;

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

pub fn render(
    options: RenderOptions,
    report: impl Fn(RenderProgress),
) -> Result<String, String> {
    let work_dir = unique_work_dir()?;
    let result = render_inner(&options, &work_dir, &report);
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

/// 連結したセグメントを入力として渡すための ffmpeg 引数を組み立てる。
///
/// concat デマルチプレクサはリストファイルを要求するので、作った場合は
/// 処理が終わるまで消えないようパスも返す。
fn build_concat_input(
    segments: &[PathBuf],
    profile: &CodecProfile,
    work_dir: &Path,
) -> Result<(Vec<String>, Option<PathBuf>), String> {
    match profile.strategy {
        ConcatStrategy::MpegTs { .. } => {
            let joined = segments
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("|");
            Ok((vec!["-i".to_string(), format!("concat:{joined}")], None))
        }
        ConcatStrategy::ConcatDemuxer => {
            let list_path = work_dir.join("segments.txt");
            let body = segments
                .iter()
                .map(|p| format!("file '{}'", p.to_string_lossy().replace('\\', "/")))
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(&list_path, body)
                .map_err(|e| format!("連結リストを書き出せません: {e}"))?;
            Ok((
                vec![
                    "-f".to_string(),
                    "concat".to_string(),
                    "-safe".to_string(),
                    "0".to_string(),
                    "-i".to_string(),
                    list_path.to_string_lossy().to_string(),
                ],
                Some(list_path),
            ))
        }
    }
}

fn render_inner(
    options: &RenderOptions,
    work_dir: &Path,
    report: &impl Fn(RenderProgress),
) -> Result<String, String> {
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
    report(RenderProgress { ratio: 0.0, message: "タイムコードを生成しています".into() });
    let ltc_path = write_ltc_file(options, config, start_frames, work_dir)?;

    let profile = codec_profile(&options.video_codec)?;
    let extension = profile.segment_extension;

    // 1. 本編をストリームコピーでセグメント化
    report(RenderProgress { ratio: WEIGHT_LTC, message: "映像を読み出しています".into() });
    let main_segment = work_dir.join(format!("main.{extension}"));
    let source_duration_us = options.total_samples * 1_000_000 / options.sample_rate as u64;
    remux_main_segment(
        &options.input_path,
        &profile,
        &main_segment,
        source_duration_us,
        |done| {
            report(RenderProgress {
                ratio: WEIGHT_LTC + WEIGHT_REMUX * done,
                message: "映像を読み出しています".into(),
            });
        },
    )?;

    // 2. 必要なら黒セグメントを作る
    let black_base = WEIGHT_LTC + WEIGHT_REMUX;
    let mut segments: Vec<PathBuf> = Vec::new();

    // プリとポストで進捗の範囲を折半する
    let black_parts =
        (options.preroll_frames > 0) as u32 + (options.postroll_frames > 0) as u32;
    let mut finished_parts = 0u32;
    let black_progress = |done: f64, finished: u32| {
        if black_parts == 0 {
            return;
        }
        let share = (finished as f64 + done) / black_parts as f64;
        report(RenderProgress {
            ratio: black_base + WEIGHT_BLACK * share,
            message: "前後の黒を作成しています".into(),
        });
    };

    if black_parts > 0 {
        black_progress(0.0, 0);
    }

    if options.preroll_frames > 0 {
        let pre = work_dir.join(format!("pre.{extension}"));
        encode_black_segment(options, &profile, options.preroll_frames, &pre, |done| {
            black_progress(done, finished_parts)
        })?;
        finished_parts += 1;
        segments.push(pre);
    }
    segments.push(main_segment);
    if options.postroll_frames > 0 {
        let post = work_dir.join(format!("post.{extension}"));
        encode_black_segment(options, &profile, options.postroll_frames, &post, |done| {
            black_progress(done, finished_parts)
        })?;
        segments.push(post);
    }

    // 連結の指定方法は方式によって変わる
    let (concat_args, _list_file) = build_concat_input(&segments, &profile, work_dir)?;

    // 3. 連結した映像 + LTC + 元音声 を MOV に多重化する
    let sample_rate = options.sample_rate.to_string();
    let filter = build_audio_filter(options);
    let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
    args.extend(concat_args);
    args.extend([
        // LTCはヘッダの無い生データなので形式を明示する
        "-f".into(),
        "f32le".into(),
        "-ar".into(),
        sample_rate.clone(),
        "-ac".into(),
        "1".into(),
        "-i".into(),
        ltc_path.to_string_lossy().to_string(),
    ]);

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

    let base = WEIGHT_LTC + WEIGHT_REMUX + WEIGHT_BLACK;
    let output_duration_us = options.total_samples * 1_000_000 / options.sample_rate as u64;

    report(RenderProgress { ratio: base, message: "書き出しています".into() });

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_with_progress(&arg_refs, output_duration_us, |done| {
        report(RenderProgress {
            ratio: base + WEIGHT_OUTPUT * done,
            message: "書き出しています".into(),
        });
    })?;

    report(RenderProgress { ratio: 1.0, message: "完了".into() });
    Ok(options.output_path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::run;

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
        render(options, |_| {}).expect("レンダリングに失敗しました");

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

        render(options_for(&source, &out, 30, 0, 1, total_samples), |_| {})
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

        render(options_for(&source, &out, 0, 0, 1, total_samples), |_| {})
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

        render(options_for(&source, &out, 30, 0, 0, total_samples), |_| {})
            .expect("音声トラックが無い素材でレンダリングに失敗しました");

        let json = probe_json(&out);
        assert!(json.contains("\"channels\": 2"), "{json}");

        for f in [source, out] {
            let _ = fs::remove_file(f);
        }
    }

    /// ProResは業務マスターの主要形式。MPEG-TSに入れられないため、
    /// H.264とは別の連結方式(concatデマルチプレクサ)を通る。
    /// ffmpegはProResをTSへ入れようとしても終了コード0のまま
    /// データストリームとして書き出してしまうため、経路を誤ると
    /// 「成功したように見えて映像が消える」形で壊れる。
    #[test]
    fn renders_prores_with_black_padding() {
        let source = temp_path("tcmix_prores_src.mov");
        let source_str = source.to_string_lossy().to_string();
        let output = run(
            "ffmpeg",
            &[
                "-y", "-v", "error",
                "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30000/1001:duration=2",
                "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=2",
                "-c:v", "prores_ks", "-profile:v", "3", "-pix_fmt", "yuv422p10le",
                "-c:a", "pcm_s16le", &source_str,
            ],
        )
        .expect("ffmpegを実行できません");
        assert!(output.status.success(), "ProRes素材の生成に失敗しました");

        let total_frames = 60 + 30;
        let total_samples = total_samples_for(total_frames);
        let out = temp_path("tcmix_prores_out.mov");

        let mut options = options_for(&source, &out, 30, 0, 1, total_samples);
        options.video_codec = "prores".into();
        options.pixel_format = "yuv422p10le".into();

        render(options, |_| {}).expect("ProResのレンダリングに失敗しました");

        let json = probe_json(&out);
        assert!(json.contains("\"codec_name\": \"prores\""), "映像がProResで残っていません: {json}");
        assert!(json.contains("\"codec_type\": \"video\""), "映像トラックが消えています: {json}");
        assert!(json.contains("\"codec_name\": \"pcm_s24le\""), "{json}");

        let decode = run("ffmpeg", &["-v", "error", "-i", &out.to_string_lossy(), "-f", "null", "-"])
            .expect("ffmpegを実行できません");
        let stderr = String::from_utf8_lossy(&decode.stderr);
        assert!(stderr.trim().is_empty(), "復号エラーが出ました: {stderr}");

        for f in [source, out] {
            let _ = fs::remove_file(f);
        }
    }

    #[test]
    fn rejects_padding_for_unsupported_codec() {
        let result = codec_profile("vp9");
        assert!(result.is_err());
        let message = result.unwrap_err();
        assert!(message.contains("0にすれば"), "対処方法が案内されていません: {message}");
    }
}
