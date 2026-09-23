// アイコンのSVGを生成する。
// 矩形波をstrokeで描くと結合部に描画アーティファクトが出るため、塗りの矩形を並べて構成する。
import { writeFileSync } from 'node:fs';

const SIZE = 1024;
const T = 44;                    // 波形の太さ
const HIGH = 306;                // 上側の水平線の中心
const LOW = 470;                 // 下側の水平線の中心
const X0 = 170;
const X1 = 854;

// 高低の並び。
// 連結棒は幅Tで遷移点をまたぐため、ランの幅がTの2倍を下回ると塗り潰れて波形に見えなくなる。
const RUNS = [
  ['h', 144], ['l', 96], ['h', 96], ['l', 144], ['h', 96], ['l', 108],
];

const rects = [];
let x = X0;
let previous = null;

for (const [level, width] of RUNS) {
  const centerY = level === 'h' ? HIGH : LOW;
  rects.push({ x, y: centerY - T / 2, w: width, h: T });

  // 高さが変わる位置に縦の連結棒を置く
  if (previous && previous !== level) {
    rects.push({ x: x - T / 2, y: HIGH - T / 2, w: T, h: LOW - HIGH + T });
  }
  previous = level;
  x += width;
}
if (x !== X1) rects[rects.length - 1].w += X1 - x;

// Rch: 音源の波形。棒の数を絞り、小さいサイズでも分離して見えるようにする
const BAR_W = 68;
const BAR_CENTER = 716;
const BAR_HEIGHTS = [96, 208, 300, 152, 264, 110];
const gap = (X1 - X0 - BAR_W * BAR_HEIGHTS.length) / (BAR_HEIGHTS.length - 1);

const bars = BAR_HEIGHTS.map((h, i) => ({
  x: X0 + i * (BAR_W + gap),
  y: BAR_CENTER - h / 2,
  w: BAR_W,
  h,
  r: BAR_W / 2,
}));

const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${SIZE}" height="${SIZE}" viewBox="0 0 ${SIZE} ${SIZE}">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#6366f1"/>
      <stop offset="55%" stop-color="#8b5cf6"/>
      <stop offset="100%" stop-color="#a855f7"/>
    </linearGradient>
  </defs>

  <rect width="${SIZE}" height="${SIZE}" rx="224" fill="#0b0e1f"/>
  <rect x="24" y="24" width="${SIZE - 48}" height="${SIZE - 48}" rx="204" fill="url(#bg)"/>

  <!-- Lch: LTCの矩形波 -->
  <g fill="#ffffff">
${rects.map(r => `    <rect x="${r.x}" y="${r.y}" width="${r.w}" height="${r.h}"/>`).join('\n')}
  </g>

  <!-- Rch: 音源の波形 -->
  <g fill="#ffffff" fill-opacity="0.88">
${bars.map(b => `    <rect x="${b.x.toFixed(1)}" y="${b.y}" width="${b.w}" height="${b.h}" rx="${b.r}"/>`).join('\n')}
  </g>
</svg>
`;

writeFileSync(new URL('./icon.svg', import.meta.url), svg, 'utf8');
console.log(`矩形波: ${rects.length}個 / 波形バー: ${bars.length}本`);
