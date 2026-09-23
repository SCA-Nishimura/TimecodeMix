// USBで配布するためのポータブル版を作る。
//
// インストーラは作らない。実行ファイルと同梱ffmpegを1つのフォルダにまとめるだけで、
// 受け取った側はフォルダごとコピーして exe を起動すれば使える。
// インストール不要・管理者権限不要で、USB経由ならSmartScreenの警告も出ない。
//
//   使い方: node scripts/package-portable.mjs

import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, rmSync, statSync, existsSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const desktopDir = join(repoRoot, 'desktop');
const releaseDir = join(desktopDir, 'src-tauri', 'target', 'release');
const outputDir = join(repoRoot, 'dist-portable', 'TimecodeMix');

const isWindows = process.platform === 'win32';
const exe = isWindows ? '.exe' : '';
const PAYLOAD = [`timecode-mix${exe}`, `ffmpeg${exe}`, `ffprobe${exe}`];

const README = `TimecodeMix

■ 使い方
  このフォルダをまるごとパソコンにコピーしてから、
  timecode-mix${exe} をダブルクリックしてください。
  インストールは不要です。

■ 注意
  3つのファイルは同じフォルダに入れたまま使ってください。
  timecode-mix${exe} だけを取り出すと動作しません。
`;

function step(message) {
  console.log(`\n== ${message}`);
}

step('ffmpeg / ffprobe を配置');
execFileSync(process.execPath, [join(repoRoot, 'scripts', 'setup-ffmpeg.mjs')], {
  stdio: 'inherit',
});

step('リリースビルド (数分かかります)');
// shell: true を使うと引数がエスケープされないため、Windowsでは npm.cmd を直接叩く
execFileSync(isWindows ? 'npm.cmd' : 'npm', ['run', 'tauri', 'build', '--', '--no-bundle'], {
  cwd: desktopDir,
  stdio: 'inherit',
});

step('配布フォルダを作成');
rmSync(outputDir, { recursive: true, force: true });
mkdirSync(outputDir, { recursive: true });

let totalBytes = 0;
for (const name of PAYLOAD) {
  const source = join(releaseDir, name);
  if (!existsSync(source)) {
    console.error(`\n[エラー] ${name} が見つかりません: ${source}`);
    process.exit(1);
  }
  copyFileSync(source, join(outputDir, name));
  const bytes = statSync(source).size;
  totalBytes += bytes;
  console.log(`  ${name.padEnd(20)} ${(bytes / 1024 / 1024).toFixed(0).padStart(4)} MB`);
}

// Windowsのメモ帳は既定でShift-JISとして読もうとするため、BOMを付けてUTF-8と判別させる。
// BOMが無いと受け取った側で文字化けする。
writeFileSync(join(outputDir, 'はじめにお読みください.txt'), '﻿' + README, 'utf8');

console.log(`\n  合計 ${(totalBytes / 1024 / 1024).toFixed(0)} MB`);
console.log(`\n配布フォルダ: ${outputDir}`);
console.log('このフォルダをUSBにコピーして配布してください。');
