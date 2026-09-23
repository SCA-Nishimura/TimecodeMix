import { FRAME_RATES } from './ltc.ts';

/**
 * Result of inspecting a video file's ffprobe output.
 */
export interface VideoDetection {
  /** Matching id in FRAME_RATES, or null when the rate has no LTC equivalent */
  fpsId: string | null;
  /** Frame rate as detected, for display (e.g. "29.97") */
  detectedFpsLabel: string;
  /** True when the source rate exceeded 30fps and was halved to fit LTC */
  isHalved: boolean;
  isVariableFrameRate: boolean;
  /** Start timecode embedded in the source (tmcd track), if any */
  embeddedTimecode: string | null;
  embeddedIsDropFrame: boolean;
  /** Message shown to the user explaining the suggested frame rate */
  recommendation: string;
  warnings: string[];
}

/**
 * Frame rates are compared as exact rationals so that 29.97 (30000/1001)
 * is never confused with a true 30.
 */
const RATE_TABLE: {
  num: number;
  den: number;
  label: string;
  fpsId: string;
  halved?: boolean;
}[] = [
  { num: 24000, den: 1001, label: '23.976', fpsId: '23.976' },
  { num: 24, den: 1, label: '24', fpsId: '24' },
  { num: 25, den: 1, label: '25', fpsId: '25' },
  { num: 30000, den: 1001, label: '29.97', fpsId: '29.97-ndf' },
  { num: 30, den: 1, label: '30', fpsId: '30-ndf' },
  // LTC is only specified up to 30fps, so higher rates take half the source rate.
  { num: 48000, den: 1001, label: '47.95', fpsId: '23.976', halved: true },
  { num: 48, den: 1, label: '48', fpsId: '24', halved: true },
  { num: 50, den: 1, label: '50', fpsId: '25', halved: true },
  { num: 60000, den: 1001, label: '59.94', fpsId: '29.97-ndf', halved: true },
  { num: 60, den: 1, label: '60', fpsId: '30-ndf', halved: true },
];

function parseRational(value: string | undefined): [number, number] {
  if (!value) return [0, 1];
  const [num, den] = value.split('/').map(Number);
  return [num || 0, den || 1];
}

export function detectFromProbe(probe: any): VideoDetection {
  const warnings: string[] = [];
  const stream = (probe?.streams ?? []).find((s: any) => s.codec_type === 'video');

  if (!stream) {
    return {
      fpsId: null,
      detectedFpsLabel: '-',
      isHalved: false,
      isVariableFrameRate: false,
      embeddedTimecode: null,
      embeddedIsDropFrame: false,
      recommendation: '映像トラックが見つかりませんでした。',
      warnings,
    };
  }

  const [rNum, rDen] = parseRational(stream.r_frame_rate);
  const [aNum, aDen] = parseRational(stream.avg_frame_rate);
  const nominalFps = rNum / rDen;
  const averageFps = aNum / aDen;

  const isVariableFrameRate =
    averageFps > 0 && Math.abs(nominalFps - averageFps) / nominalFps > 0.01;
  if (isVariableFrameRate) {
    warnings.push(
      `可変フレームレート(VFR)の可能性があります(公称 ${nominalFps.toFixed(3)}fps / 実測平均 ${averageFps.toFixed(3)}fps)。` +
        'タイムコードがズレる恐れがあるため、固定フレームレートに変換してから使用することを推奨します。'
    );
  }

  // Drop frame is written into the tmcd track as a ';' separator.
  const embeddedTimecode: string | null =
    stream.tags?.timecode ?? probe?.format?.tags?.timecode ?? null;
  const embeddedIsDropFrame = !!embeddedTimecode && embeddedTimecode.includes(';');

  const match = RATE_TABLE.find(entry => entry.num === rNum && entry.den === rDen);

  if (!match) {
    return {
      fpsId: null,
      detectedFpsLabel: nominalFps ? nominalFps.toFixed(3) : '-',
      isHalved: false,
      isVariableFrameRate,
      embeddedTimecode,
      embeddedIsDropFrame,
      recommendation:
        `この動画のフレームレート(${nominalFps.toFixed(3)}fps)はLTCの規格に該当しませんでした。手動で選択してください。`,
      warnings,
    };
  }

  let fpsId = match.fpsId;
  if (embeddedIsDropFrame && fpsId === '29.97-ndf') {
    fpsId = '29.97-df';
  }

  const label = FRAME_RATES.find(f => f.id === fpsId)?.label ?? fpsId;

  let recommendation = match.halved
    ? `埋め込まれた動画のFrameRateは ${match.label}FPS でした。LTCは30FPSまでの規格のため、その半分にあたる ${label} を推奨します。`
    : `埋め込まれた動画のFrameRateは ${match.label}FPS だったので、${label} を推奨します。`;

  if (embeddedTimecode) {
    recommendation +=
      `\nまた元素材に開始タイムコード ${embeddedTimecode}` +
      `${embeddedIsDropFrame ? '(ドロップフレーム)' : ''} が記録されていたため、開始TCの初期値に設定しました。`;
  }

  return {
    fpsId,
    detectedFpsLabel: match.label,
    isHalved: !!match.halved,
    isVariableFrameRate,
    embeddedTimecode,
    embeddedIsDropFrame,
    recommendation,
    warnings,
  };
}

/**
 * Preroll and postroll must land on whole video frames, otherwise the padded
 * video and the generated audio end up with slightly different durations.
 */
export function quantizeToFrames(seconds: number, actualFps: number): { frames: number; seconds: number } {
  const frames = Math.round(seconds * actualFps);
  return { frames, seconds: frames / actualFps };
}
