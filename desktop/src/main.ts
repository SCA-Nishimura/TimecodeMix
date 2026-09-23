import { FRAME_RATES, generateLtcBuffer, parseTimecode } from '../../core/ltc.ts';
import { quantizeToFrames } from '../../core/video.ts';

// 起動確認用: core/ がデスクトップ側からも読めているかを画面に出す
const status = document.getElementById('status') as HTMLElement;

const config = FRAME_RATES.find(f => f.id === '29.97-ndf')!;
const startFrames = parseTimecode('01:00:00:00', config);
const ltc = generateLtcBuffer(1, 48000, startFrames, -6, config);
const preroll = quantizeToFrames(3, config.actualFps);

status.innerHTML = `
  <h2>core/ の読み込み確認</h2>
  <dl>
    <dt>利用可能なフレームレート</dt>
    <dd>${FRAME_RATES.length} 種類 (${FRAME_RATES.map(f => f.label).join(' / ')})</dd>
    <dt>parseTimecode('01:00:00:00')</dt>
    <dd>${startFrames.toLocaleString()} フレーム</dd>
    <dt>generateLtcBuffer(1秒 @48kHz)</dt>
    <dd>${ltc.length.toLocaleString()} サンプル</dd>
    <dt>quantizeToFrames(3秒 @29.97fps)</dt>
    <dd>${preroll.frames} フレーム (${preroll.seconds.toFixed(4)} 秒)</dd>
  </dl>
`;
