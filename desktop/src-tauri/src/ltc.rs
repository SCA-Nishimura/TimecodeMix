//! LTC (Linear Timecode) の生成。SMPTE 12M 準拠。
//!
//! Web版の `core/ltc.ts` と同じアルゴリズムを実装している。
//! デスクトップ版でTypeScript側に生成させると、60分素材で879MBのデータを
//! IPC越しに運ぶことになり実用にならないため、こちらはRustで生成する。
//!
//! 両者が食い違わないよう、出力が一致することを tests で検証している。
//! 片方だけ変更するとテストが落ちる。

use std::io::{self, Write};

#[derive(Debug, Clone, Copy)]
pub struct FrameRateConfig {
    pub id: &'static str,
    pub timecode_fps: u32,
    pub actual_fps: f64,
    pub is_drop_frame: bool,
}

pub const FRAME_RATES: &[FrameRateConfig] = &[
    FrameRateConfig { id: "30-ndf", timecode_fps: 30, actual_fps: 30.0, is_drop_frame: false },
    FrameRateConfig { id: "30-df", timecode_fps: 30, actual_fps: 30000.0 / 1001.0, is_drop_frame: true },
    FrameRateConfig { id: "29.97-ndf", timecode_fps: 30, actual_fps: 30000.0 / 1001.0, is_drop_frame: false },
    FrameRateConfig { id: "29.97-df", timecode_fps: 30, actual_fps: 30000.0 / 1001.0, is_drop_frame: true },
    FrameRateConfig { id: "25", timecode_fps: 25, actual_fps: 25.0, is_drop_frame: false },
    FrameRateConfig { id: "24", timecode_fps: 24, actual_fps: 24.0, is_drop_frame: false },
    FrameRateConfig { id: "23.976", timecode_fps: 24, actual_fps: 24000.0 / 1001.0, is_drop_frame: false },
];

pub fn find_frame_rate(id: &str) -> Option<&'static FrameRateConfig> {
    FRAME_RATES.iter().find(|f| f.id == id)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Timecode {
    pub hours: i64,
    pub minutes: i64,
    pub seconds: i64,
    pub frames: i64,
}

pub fn frame_to_timecode(total_frames: i64, config: &FrameRateConfig) -> Timecode {
    if config.is_drop_frame {
        // 10分で17982フレーム(2フレーム×9分ぶんが欠番)を1単位として数える
        let d = total_frames.div_euclid(17982);
        let m = total_frames.rem_euclid(17982);

        let mut minutes = d * 10;
        let mut remaining = m;
        if remaining >= 1800 {
            remaining -= 1800;
            let extra_minutes = remaining / 1798 + 1;
            minutes += extra_minutes;
            remaining %= 1798;
            remaining += 2;
        }
        let seconds = remaining / 30;
        let frames = remaining % 30;
        let hours = (minutes / 60) % 24;
        minutes %= 60;

        Timecode { hours, minutes, seconds, frames }
    } else {
        let fps = config.timecode_fps as i64;
        let mut f = total_frames;

        let frames = f.rem_euclid(fps);
        f = f.div_euclid(fps);
        let seconds = f.rem_euclid(60);
        f = f.div_euclid(60);
        let minutes = f.rem_euclid(60);
        f = f.div_euclid(60);
        let hours = f.rem_euclid(24);

        Timecode { hours, minutes, seconds, frames }
    }
}

pub fn format_timecode(tc: &Timecode, is_drop_frame: bool) -> String {
    let separator = if is_drop_frame { ';' } else { ':' };
    format!(
        "{:02}:{:02}:{:02}{}{:02}",
        tc.hours, tc.minutes, tc.seconds, separator, tc.frames
    )
}

pub fn parse_timecode(text: &str, config: &FrameRateConfig) -> Result<i64, String> {
    let parts: Vec<&str> = text.trim().split([':', '.', ';', ',']).collect();
    if parts.len() != 4 {
        return Err("タイムコードの形式が正しくありません。HH:MM:SS:FF で入力してください。".into());
    }
    let mut values = [0i64; 4];
    for (i, part) in parts.iter().enumerate() {
        values[i] = part
            .parse::<i64>()
            .map_err(|_| "タイムコードに数値以外が含まれています。".to_string())?;
    }
    let [h, m, s, f] = values;

    let max_frames = config.timecode_fps as i64;
    if h < 0 || m < 0 || m >= 60 || s < 0 || s >= 60 || f < 0 || f >= max_frames {
        return Err(format!(
            "値が範囲外です。フレームは0〜{}、分と秒は0〜59で指定してください。",
            max_frames - 1
        ));
    }

    if config.is_drop_frame {
        let total_minutes = h * 60 + m;
        let dropped = 2 * (total_minutes - total_minutes / 10);
        if m % 10 != 0 && s == 0 && (f == 0 || f == 1) {
            return Err("このタイムコードは欠番フレームです(ドロップフレームで存在しません)。".into());
        }
        Ok((h * 3600 + m * 60 + s) * 30 + f - dropped)
    } else {
        Ok((h * 3600 + m * 60 + s) * max_frames + f)
    }
}

/// 1フレーム分の80ビットを組み立てる
pub fn generate_ltc_bits(total_frame_index: i64, config: &FrameRateConfig) -> [u8; 80] {
    let tc = frame_to_timecode(total_frame_index, config);
    let mut bits = [0u8; 80];

    let write_bcd = |bits: &mut [u8; 80], value: i64, start: usize, count: usize| {
        for i in 0..count {
            bits[start + i] = ((value >> i) & 1) as u8;
        }
    };

    write_bcd(&mut bits, tc.frames % 10, 0, 4);
    write_bcd(&mut bits, tc.frames / 10, 8, 2);
    bits[10] = u8::from(config.is_drop_frame); // ドロップフレームフラグ
    bits[11] = 0; // カラーフレームフラグ

    write_bcd(&mut bits, tc.seconds % 10, 16, 4);
    write_bcd(&mut bits, tc.seconds / 10, 24, 3);
    // bit 27 はパリティ。最後に決める

    write_bcd(&mut bits, tc.minutes % 10, 32, 4);
    write_bcd(&mut bits, tc.minutes / 10, 40, 3);
    bits[43] = 0; // BGF0

    write_bcd(&mut bits, tc.hours % 10, 48, 4);
    write_bcd(&mut bits, tc.hours / 10, 56, 2);
    bits[58] = 0; // BGF1
    bits[59] = 0; // BGF2

    // シンクワード (64-79): 0011 1111 1111 1101
    bits[64] = 0;
    bits[65] = 0;
    for bit in bits.iter_mut().take(78).skip(66) {
        *bit = 1;
    }
    bits[78] = 0;
    bits[79] = 1;

    // 80ビット中の1の個数が偶数になるようbit 27で調整する
    let ones = bits
        .iter()
        .enumerate()
        .filter(|(i, &b)| *i != 27 && b == 1)
        .count();
    bits[27] = u8::from(ones % 2 == 1);

    bits
}

/// LTC波形をf32leの生データとして書き出す。
///
/// TS版は全体をFloat32Arrayに確保するが、こちらはフレーム単位で書き出すため
/// 尺に関わらずメモリ使用量が一定になる。
pub fn write_ltc<W: Write>(
    writer: &mut W,
    duration_seconds: f64,
    sample_rate: u32,
    start_frames_offset: i64,
    level_dbfs: f64,
    config: &FrameRateConfig,
) -> io::Result<u64> {
    let total_samples = (duration_seconds * sample_rate as f64).floor() as i64;
    let total_frames = (duration_seconds * config.actual_fps).ceil() as i64;
    let amplitude = 10f64.powf(level_dbfs / 20.0);

    let mut current_level: f64 = -1.0;
    let mut frame_buffer: Vec<f32> = Vec::new();
    let mut written: u64 = 0;

    for f in 0..total_frames {
        let frame_start = (f as f64 * sample_rate as f64 / config.actual_fps).floor() as i64;
        let frame_end = ((f + 1) as f64 * sample_rate as f64 / config.actual_fps).floor() as i64;
        let samples_in_frame = frame_end - frame_start;
        if samples_in_frame <= 0 {
            continue;
        }

        frame_buffer.clear();
        frame_buffer.resize(samples_in_frame as usize, 0.0);

        let bits = generate_ltc_bits(f + start_frames_offset, config);

        for (b, &bit) in bits.iter().enumerate() {
            let b = b as i64;
            let bit_start = (b as f64 * samples_in_frame as f64 / 80.0).floor() as i64;
            let bit_end = ((b + 1) as f64 * samples_in_frame as f64 / 80.0)
                .floor()
                .min(samples_in_frame as f64) as i64;
            let bit_mid = ((b as f64 + 0.5) * samples_in_frame as f64 / 80.0).floor() as i64;

            // Bi-phase Mark Code: ビットの先頭で必ず反転する
            current_level = -current_level;

            if bit == 1 {
                // 1 はビットの中央でもう一度反転する
                for i in bit_start..bit_mid {
                    frame_buffer[i as usize] = (current_level * amplitude) as f32;
                }
                current_level = -current_level;
                for i in bit_mid..bit_end {
                    frame_buffer[i as usize] = (current_level * amplitude) as f32;
                }
            } else {
                for i in bit_start..bit_end {
                    frame_buffer[i as usize] = (current_level * amplitude) as f32;
                }
            }
        }

        // 指定尺を超える分は書かない
        if frame_start >= total_samples {
            break;
        }
        let writable = (total_samples - frame_start).min(samples_in_frame) as usize;

        let mut bytes = Vec::with_capacity(writable * 4);
        for value in &frame_buffer[..writable] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        writer.write_all(&bytes)?;
        written += writable as u64;
    }

    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    /// core/ltc.ts が生成したLTCのSHA-256。
    /// 生成方法: scripts/gen-reference-hashes.ts を bun で実行する。
    ///
    /// Web版とデスクトップ版でLTCの実装が分かれているため、この表が両者の一致を担保する。
    /// 片方だけアルゴリズムを変更するとここが落ちる。意図的な変更なら期待値を再生成すること。
    ///
    /// (fpsId, 尺(秒), サンプリングレート, 開始フレーム, レベル(dBFS), サンプル数, SHA-256)
    const REFERENCE: &[(&str, f64, u32, i64, f64, u64, &str)] = &[
        ("30-ndf", 2.0, 48000, 108000, -6.0, 96000, "97cda697f4ad8d692531b6875229fbf71a9478be5dc7b13328917681199cf208"),
        ("30-df", 2.0, 48000, 107892, -6.0, 96000, "fedd2983eb5ab472a1df86a18fc30db8fb70bb11fa12b391b87d3e690dcfaf79"),
        ("29.97-ndf", 2.0, 48000, 108000, -6.0, 96000, "e74afcbde1eb86f75d7dd25ea390e692b993e4410227fe6217622d143872968c"),
        ("29.97-df", 2.0, 48000, 107892, -6.0, 96000, "fedd2983eb5ab472a1df86a18fc30db8fb70bb11fa12b391b87d3e690dcfaf79"),
        ("25", 2.0, 48000, 90000, -6.0, 96000, "f7d6e499a658da02c2edf7200c73e5d0408dd7747bb3cda89b3b2962af9f31bf"),
        ("24", 2.0, 48000, 86400, -6.0, 96000, "1f850c6afd4b13c19ea1058569169e8a881912aa972eb2235952c4862c4a61e5"),
        ("23.976", 2.0, 48000, 86400, -6.0, 96000, "5af852ef1e568279b44087687de5a33e4a2216df8e8082ca7eef2284f118a071"),
        // ドロップフレームの10分境界をまたぐ
        ("29.97-df", 3.0, 48000, 17922, -6.0, 144000, "3f4fc4c4a1e44659da405c1508b1b788f544b3a006ea9e27dc7e6d408cb5d0b8"),
        ("30-df", 3.0, 48000, 1770, -6.0, 144000, "e169251ed9e1f22ab1c9598fd83a0546363c2c2f30d44fe954cf0a64ad67c86a"),
        // レベルの上下端
        ("29.97-ndf", 1.0, 48000, 108000, 0.0, 48000, "d5fc672d14f60a08054bb55f73b874edc988506d48a98ac401bcaf3d32fe637e"),
        ("29.97-ndf", 1.0, 48000, 108000, -18.0, 48000, "2011b3927a0a534e703bef3358789df1fa8295d18dab93f0fa91264e32fb763a"),
        // 別のサンプリングレート
        ("25", 2.0, 44100, 900000, -6.0, 88200, "41057fa09786a47ba55a49b24676f9bb1a8aac6d7987439e0d8f7d7a90e0a431"),
        ("23.976", 2.0, 96000, 0, -12.0, 192000, "9d6beb1a8d757e0ab5f1be435a89f9a4c982ced1808f33f2098787f3da9d4806"),
        // 24時間で繰り上がる位置
        ("24", 1.5, 48000, 2073576, -6.0, 72000, "cc206fd5a89a27f6da3235e8506e7ba73ab002456a62b2af77f74cc00991bc81"),
        // 尺が整数フレームで割り切れない
        ("29.97-ndf", 0.7333, 48000, 108000, -6.0, 35198, "61a64e897489f8ba006df62755785902c256d8e5fec76235d3cb664d2f30c3bc"),
    ];

    #[test]
    fn matches_typescript_implementation() {
        let mut failures = Vec::new();

        for &(fps_id, duration, sample_rate, start_frames, level, expected_samples, expected_hash) in
            REFERENCE
        {
            let config = find_frame_rate(fps_id).expect("フレームレート定義がありません");
            let mut buffer = Vec::new();
            let written =
                write_ltc(&mut buffer, duration, sample_rate, start_frames, level, config)
                    .expect("LTCを書き出せませんでした");

            let hash = format!("{:x}", Sha256::digest(&buffer));

            if written != expected_samples {
                failures.push(format!(
                    "{fps_id} {duration}秒: サンプル数が違います (Rust={written} / TS={expected_samples})"
                ));
            } else if hash != expected_hash {
                failures.push(format!(
                    "{fps_id} {duration}秒 開始{start_frames} レベル{level}: 波形が一致しません\n      Rust={hash}\n      TS  ={expected_hash}"
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "TypeScript版と出力が一致しません:\n  - {}",
            failures.join("\n  - ")
        );
    }

    #[test]
    fn drop_frame_skips_numbers_at_minute_boundaries() {
        let config = find_frame_rate("29.97-df").unwrap();
        // 00:00:59:29 の次は 00:01:00:02 (00と01は欠番)
        let at_59 = parse_timecode("00:00:59:29", config).unwrap();
        let next = frame_to_timecode(at_59 + 1, config);
        assert_eq!(
            format_timecode(&next, true),
            "00:01:00;02",
            "ドロップフレームの欠番処理が正しくありません"
        );

        // 10分の倍数では欠番にしない
        let at_9_59 = parse_timecode("00:09:59:29", config).unwrap();
        let next_10 = frame_to_timecode(at_9_59 + 1, config);
        assert_eq!(format_timecode(&next_10, true), "00:10:00;00");
    }

    #[test]
    fn non_drop_frame_counts_every_frame() {
        let config = find_frame_rate("29.97-ndf").unwrap();
        let at_59 = parse_timecode("00:00:59:29", config).unwrap();
        let next = frame_to_timecode(at_59 + 1, config);
        assert_eq!(format_timecode(&next, false), "00:01:00:00");
    }

    #[test]
    fn parse_timecode_rejects_invalid_input() {
        let config = find_frame_rate("29.97-ndf").unwrap();
        assert!(parse_timecode("01:00:00", config).is_err(), "要素数の不足を検出できていません");
        assert!(parse_timecode("01:00:00:30", config).is_err(), "フレーム数の範囲外を検出できていません");
        assert!(parse_timecode("01:60:00:00", config).is_err(), "分の範囲外を検出できていません");
        assert!(parse_timecode("aa:bb:cc:dd", config).is_err(), "非数値を検出できていません");

        let df = find_frame_rate("29.97-df").unwrap();
        assert!(parse_timecode("00:01:00:00", df).is_err(), "欠番フレームを検出できていません");
        assert!(parse_timecode("00:10:00:00", df).is_ok(), "10分ちょうどは有効なはずです");
    }

    #[test]
    fn sync_word_and_parity_are_correct() {
        let config = find_frame_rate("29.97-ndf").unwrap();
        let bits = generate_ltc_bits(108000, config);

        // シンクワード 0011111111111101
        let sync: Vec<u8> = bits[64..80].to_vec();
        assert_eq!(sync, vec![0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1]);

        // 80ビット中の1の個数は偶数
        let ones = bits.iter().filter(|&&b| b == 1).count();
        assert_eq!(ones % 2, 0, "パリティが合っていません");
    }
}

