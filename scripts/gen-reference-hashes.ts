// TS版(core/ltc.ts)が生成するLTCのハッシュを算出し、Rust版の検証用の基準として出力する。
// アルゴリズムを意図的に変更した場合は、これを再実行して期待値を更新する。
import { createHash } from 'node:crypto';
import { FRAME_RATES, generateLtcBuffer, parseTimecode } from '../core/ltc.ts';

interface Case {
  fpsId: string;
  durationSeconds: number;
  sampleRate: number;
  startTimecode: string;
  levelDbfs: number;
}

const cases: Case[] = [
  // 全フレームレートを基本条件で
  ...FRAME_RATES.map(f => ({
    fpsId: f.id,
    durationSeconds: 2,
    sampleRate: 48000,
    startTimecode: '01:00:00:00',
    levelDbfs: -6,
  })),
  // ドロップフレームの10分境界をまたぐケース(欠番の扱いが出る)
  { fpsId: '29.97-df', durationSeconds: 3, sampleRate: 48000, startTimecode: '00:09:58:00', levelDbfs: -6 },
  { fpsId: '30-df', durationSeconds: 3, sampleRate: 48000, startTimecode: '00:00:59:00', levelDbfs: -6 },
  // レベルの上下端
  { fpsId: '29.97-ndf', durationSeconds: 1, sampleRate: 48000, startTimecode: '01:00:00:00', levelDbfs: 0 },
  { fpsId: '29.97-ndf', durationSeconds: 1, sampleRate: 48000, startTimecode: '01:00:00:00', levelDbfs: -18 },
  // 別のサンプリングレート
  { fpsId: '25', durationSeconds: 2, sampleRate: 44100, startTimecode: '10:00:00:00', levelDbfs: -6 },
  { fpsId: '23.976', durationSeconds: 2, sampleRate: 96000, startTimecode: '00:00:00:00', levelDbfs: -12 },
  // 時間が繰り上がる位置
  { fpsId: '24', durationSeconds: 1.5, sampleRate: 48000, startTimecode: '23:59:59:00', levelDbfs: -6 },
  // 端数の出る尺
  { fpsId: '29.97-ndf', durationSeconds: 0.7333, sampleRate: 48000, startTimecode: '01:00:00:00', levelDbfs: -6 },
];

const rows = cases.map(c => {
  const config = FRAME_RATES.find(f => f.id === c.fpsId)!;
  const startFrames = parseTimecode(c.startTimecode, config);
  const buffer = generateLtcBuffer(c.durationSeconds, c.sampleRate, startFrames, c.levelDbfs, config);
  const bytes = new Uint8Array(buffer.buffer, 0, buffer.length * 4);
  const hash = createHash('sha256').update(bytes).digest('hex');
  return { ...c, startFrames, samples: buffer.length, hash };
});

// Rustのテストにそのまま貼れる形で出力する
console.log('// core/ltc.ts の出力から生成した期待値');
console.log('// 再生成: bun gen-reference-hashes.ts');
console.log('const REFERENCE: &[(&str, f64, u32, i64, f64, u64, &str)] = &[');
for (const r of rows) {
  console.log(
    `    ("${r.fpsId}", ${r.durationSeconds}, ${r.sampleRate}, ${r.startFrames}, ${r.levelDbfs}.0, ${r.samples}, "${r.hash}"),`
  );
}
console.log('];');
