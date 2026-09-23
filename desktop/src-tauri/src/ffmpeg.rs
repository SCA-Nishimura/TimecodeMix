use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Windows で子プロセスのコンソールウィンドウが一瞬表示されるのを防ぐ
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// ffmpeg / ffprobe の場所を解決する。
///
/// 配布時は実行ファイルと同じディレクトリに同梱されたものを使い、
/// 開発時はそれが無いので PATH 上のものにフォールバックする。
fn resolve_tool(name: &str) -> PathBuf {
    let file_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let bundled = dir.join(&file_name);
            if bundled.is_file() {
                return bundled;
            }
        }
    }

    PathBuf::from(name)
}

/// ffmpeg を起動し、進捗を逐次コールバックへ渡しながら完了を待つ。
///
/// `-progress pipe:1` で流れてくる `out_time_us`(出力済みの再生位置)を
/// 全体の尺で割って 0.0〜1.0 の比率にして報告する。
///
/// 映像のフレーム数(`frame=`)は使わない。ストリームコピーでは映像だけが
/// 先に進んでしまい、音声処理の進み具合を反映しないため。
pub fn run_with_progress(
    args: &[&str],
    total_duration_us: u64,
    mut on_progress: impl FnMut(f64),
) -> Result<std::process::ExitStatus, String> {
    let mut command = Command::new(resolve_tool("ffmpeg"));
    command
        .args(["-progress", "pipe:1", "-nostats"])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command
        .spawn()
        .map_err(|e| format!("ffmpeg を実行できませんでした。 ({e})"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "ffmpeg の出力を読み取れませんでした。".to_string())?;

    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(value) = line.strip_prefix("out_time_us=") {
            if total_duration_us == 0 {
                continue;
            }
            // 処理開始直後は N/A が来ることがある
            if let Ok(microseconds) = value.trim().parse::<u64>() {
                on_progress((microseconds as f64 / total_duration_us as f64).clamp(0.0, 1.0));
            }
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("ffmpeg の終了を待てませんでした。 ({e})"))?;

    if !output.status.success() {
        return Err(format!(
            "書き出しに失敗しました: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(output.status)
}

pub fn run(tool: &str, args: &[&str]) -> Result<Output, String> {
    let mut command = Command::new(resolve_tool(tool));
    command.args(args);

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    command.output().map_err(|e| {
        format!("{tool} を実行できませんでした。インストールされているか確認してください。 ({e})")
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegStatus {
    pub version: String,
    /// アプリに同梱されたものを使っているか (false ならPATH上のもの)
    pub bundled: bool,
    pub path: String,
}

/// ffprobe が利用可能かを確認し、バージョンと参照元を返す。
/// 参照元を返すのは、配布時に同梱バイナリが使われているかを確認できるようにするため。
pub fn check_available() -> Result<FfmpegStatus, String> {
    let resolved = resolve_tool("ffprobe");
    let output = run("ffprobe", &["-version"])?;
    if !output.status.success() {
        return Err("ffprobe の起動に失敗しました。".to_string());
    }

    let text = String::from_utf8_lossy(&output.stdout);
    Ok(FfmpegStatus {
        version: text.lines().next().unwrap_or("").to_string(),
        bundled: resolved.is_absolute(),
        path: resolved.to_string_lossy().to_string(),
    })
}

/// 映像ファイルのストリーム情報を ffprobe の JSON のまま返す。
/// 解釈は core/video.ts の detectFromProbe が行う。
pub fn probe(path: &str) -> Result<String, String> {
    let output = run(
        "ffprobe",
        &[
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
            path,
        ],
    )?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ファイルを解析できませんでした: {}", stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の動画を ffmpeg 自身に作らせる (固定のテスト素材を置かずに済ませる)
    fn make_fixture(name: &str, rate: &str) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        let output = run(
            "ffmpeg",
            &[
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("testsrc2=size=160x120:rate={rate}:duration=1"),
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p",
                path.to_str().unwrap(),
            ],
        )
        .expect("ffmpeg を実行できませんでした");
        assert!(output.status.success(), "テスト用動画の生成に失敗しました");
        path
    }

    #[test]
    fn ffprobe_is_available() {
        let status = check_available().expect("ffprobe が見つかりません");
        assert!(status.version.contains("ffprobe"), "想定外の出力: {}", status.version);
    }

    #[test]
    fn probe_reports_exact_frame_rate() {
        // 29.97 を 30 と取り違えないことが重要なので有理数のまま検証する
        let fixture = make_fixture("tcmix_probe_2997.mp4", "30000/1001");
        let json = probe(fixture.to_str().unwrap()).expect("probe に失敗しました");

        assert!(json.contains("\"r_frame_rate\": \"30000/1001\""), "{json}");
        assert!(json.contains("\"codec_type\": \"video\""));

        let _ = std::fs::remove_file(fixture);
    }

    #[test]
    fn probe_reports_embedded_timecode() {
        let source = make_fixture("tcmix_probe_tc_src.mp4", "30000/1001");
        let tagged = std::env::temp_dir().join("tcmix_probe_tc.mov");
        let output = run(
            "ffmpeg",
            &[
                "-y",
                "-v",
                "error",
                "-i",
                source.to_str().unwrap(),
                "-c",
                "copy",
                "-timecode",
                "10:00:00:00",
                tagged.to_str().unwrap(),
            ],
        )
        .expect("ffmpeg を実行できませんでした");
        assert!(output.status.success());

        let json = probe(tagged.to_str().unwrap()).expect("probe に失敗しました");
        assert!(json.contains("10:00:00:00"), "TCトラックが読めていません: {json}");

        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(tagged);
    }

    #[test]
    fn probe_rejects_missing_file() {
        let result = probe("C:/definitely/not/here.mp4");
        assert!(result.is_err(), "存在しないファイルがエラーになっていません");
    }
}
