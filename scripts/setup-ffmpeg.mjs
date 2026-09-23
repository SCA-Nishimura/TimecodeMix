// デスクトップ版に同梱する ffmpeg / ffprobe を用意する。
//
// バイナリは合計で数百MBあるためGitには入れず、ビルド前にこのスクリプトで配置する。
// Tauriのサイドカーは `<名前>-<ターゲットトリプル>` という命名を要求し、
// バンドル時にトリプル部分が取り除かれて実行ファイルの隣に置かれる。
//
//   使い方: node scripts/setup-ffmpeg.mjs

import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, statSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const binariesDir = join(repoRoot, 'desktop', 'src-tauri', 'binaries');

const isWindows = process.platform === 'win32';
const exeSuffix = isWindows ? '.exe' : '';

function fail(message) {
  console.error(`\n[エラー] ${message}\n`);
  process.exit(1);
}

/** Rustのホストターゲットトリプルを取得する */
function targetTriple() {
  try {
    const output = execFileSync('rustc', ['-vV'], { encoding: 'utf8' });
    const line = output.split('\n').find(l => l.startsWith('host:'));
    if (!line) fail('rustc の出力から host を読み取れませんでした。');
    return line.replace('host:', '').trim();
  } catch {
    fail('rustc が見つかりません。Rust (rustup) をインストールしてください。');
  }
}

/** PATH上の実行ファイルの場所を調べる */
function locate(tool) {
  const finder = isWindows ? 'where' : 'which';
  try {
    const output = execFileSync(finder, [tool], { encoding: 'utf8' });
    const first = output.split('\n').map(l => l.trim()).filter(Boolean)[0];
    return first && existsSync(first) ? first : null;
  } catch {
    return null;
  }
}

const triple = targetTriple();
mkdirSync(binariesDir, { recursive: true });

console.log(`ターゲット: ${triple}`);
console.log(`配置先    : ${binariesDir}\n`);

const missing = [];
let totalBytes = 0;

for (const tool of ['ffmpeg', 'ffprobe']) {
  const source = locate(tool);
  if (!source) {
    missing.push(tool);
    continue;
  }

  const destination = join(binariesDir, `${tool}-${triple}${exeSuffix}`);
  copyFileSync(source, destination);

  const bytes = statSync(destination).size;
  totalBytes += bytes;
  console.log(`  ${tool.padEnd(8)} ${(bytes / 1024 / 1024).toFixed(0).padStart(4)} MB  <- ${source}`);
}

if (missing.length > 0) {
  fail(
    `${missing.join(' と ')} が見つかりません。\n` +
      '        ffmpeg をインストールしてから再実行してください。\n' +
      '          Windows: winget install Gyan.FFmpeg\n' +
      '          macOS  : brew install ffmpeg'
  );
}

console.log(`\n合計 ${(totalBytes / 1024 / 1024).toFixed(0)} MB を配置しました。`);
