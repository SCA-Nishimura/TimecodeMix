# TimecodeMix ⚡

[![TypeScript](https://img.shields.io/badge/Language-TypeScript-blue?style=flat-square&logo=typescript)](https://www.typescriptlang.org/)
[![Rust](https://img.shields.io/badge/Language-Rust-000000?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![Tauri](https://img.shields.io/badge/Desktop-Tauri-24C8DB?style=flat-square&logo=tauri)](https://tauri.app/)
[![Vite](https://img.shields.io/badge/Bundler-Vite-646CFF?style=flat-square&logo=vite)](https://vitejs.dev/)

**Lチャンネルにタイムコード(LTC)**、**Rチャンネルに元の音源(モノラルミックス)** を、完全にパンを振った状態で結合するツールです。

照明卓・舞台演出機器へのLTC送出、マルチカメラの同期、映像と音響システムの同期に使えます。

---

## 2つの版があります

| | **Web版** | **デスクトップ版** |
|---|---|---|
| 対応ファイル | 音声 (WAV / MP3) | **映像 (MP4 / MOV)** |
| 出力 | WAV (Stereo PCM) | MOV (PCM音声 + タイムコードトラック) |
| インストール | 不要(ブラウザで開くだけ) | 不要(フォルダをコピーするだけ) |
| 用途 | 音源にLTCを付ける | 映像にLTCを付ける |

**音声だけならWeb版が手軽です。** ブラウザで開いてファイルをドロップするだけで完結します。

- 本番: https://sca-nishimura.github.io/TimecodeMix/
- ミラー: https://timecode-mix.vercel.app/

**映像を扱う場合はデスクトップ版を使ってください。** ブラウザでは扱えるファイルサイズに限界があり、ProResなどの収録マスターは開けません(詳細は [docs/architecture.md](docs/architecture.md))。

---

## 共通の仕様

- **フレームレート**: 23.976 / 24 / 25 / 29.97(NDF・DF) / 30(NDF・DF)
- **LTCレベル**: 0 〜 -18 dBFS(既定: -6 dBFS)
- **変調方式**: Bi-phase Mark Code (BMC)、SMPTE 12M 準拠
- **出力音声**: Linear PCM(16 / 24-bit)。L = LTC、R = 元音源

> **なぜPCMなのか**: LTCは矩形波のため、AACなどの非可逆圧縮をかけると波形の立ち上がりが鈍り、LTCリーダーが読み取りに失敗します。

### ステレオ音源のモノラル化について

入力がステレオの場合、Rチャンネルに入る音源は左右の算術平均 `(L + R) / 2` になります(各チャンネルを -6 dB して加算する処理に相当)。位相によって以下の影響が出ます。

| 成分 | 影響 |
|---|---|
| 同相(センター定位) | 変化なし(0 dB) |
| 無相関(左右で異なる音) | 約 -3 dB 低下 |
| 逆相(左右反転した音) | 打ち消し合い、著しく低下または無音化 |

---

## Web版の使い方

1. **ファイルを読み込む** — `.wav` または `.mp3` をドロップ。サンプリングレートとビット深度が自動検出されます
2. **設定する**
   - **開始タイムコード** — `HH:MM:SS:FF` 形式(既定: `01:00:00:00`)
   - **プリロール / ポストロール** — 音源の前後に入れる無音の秒数。**タイムコードは無音中も連続してカウントされます**
   - **FPS / LTCレベル / 出力ビット深度**
3. **「Mix & Generate WAV」** を押すと数秒で完了します
4. **プレビューとダウンロード**

> ⚠️ **プレビュー時はLchにご注意ください。** LTCの高周波音が直接出ます。「Mute LTC (Lch)」にチェックを入れると、Rch(音源)だけを安全に試聴できます。

---

## デスクトップ版の使い方

`timecode-mix.exe` / `ffmpeg.exe` / `ffprobe.exe` の3つを**同じフォルダに入れたまま**、実行ファイルを起動してください。

1. **映像ファイルをドロップ** — MP4 / MOV
2. **解析結果を確認** — フレームレートと埋め込みタイムコードを自動検出し、**推奨するLTCのFPSを提示**します
3. **設定して「書き出し」**

### 特徴

- **映像は再エンコードしません。** 画質は完全に無劣化です
- **プリロール / ポストロールは黒画面**として前後に追加されます(黒の部分だけをエンコードするため高速)
- 出力MOVには**タイムコードトラック(tmcd)も埋め込む**ため、Premiere / DaVinci Resolve のタイムラインにもTCが表示されます

### 制約

| 項目 | 内容 |
|---|---|
| プリ/ポストロール対応コーデック | H.264 / HEVC / ProRes。それ以外は0にすれば処理できます |
| 可変フレームレート(VFR) | スマホ撮影や画面収録で発生。検出して警告しますが、正確な同期には固定フレームレートへの変換が必要です |
| 50 / 59.94 / 60fps 素材 | LTCの規格上限が30fpsのため、半分のTCを割り当てます |

---

## 照明卓へ送る場合のヒント

- **30fps NDF が扱いやすい**です。30fpsは正確に30fpsなので、TCの表示時間と実時間が完全に一致します(29.97では0.1%ずれます)
- **卓側のフレームレート設定を送出側と一致させてください**
- **プリロールを5〜10秒入れる**と、卓のLTCリーダーが余裕を持ってロックした状態で本編に入れます
- **Lchには一切の加工を加えないでください**(コンプ・EQ・リミッター・非可逆圧縮は読み取り失敗の原因になります)

---

## 開発

### Web版

```bash
bun install
bun run dev     # http://localhost:3000
bun run build   # dist/ に出力
```

### デスクトップ版

必要なもの: Rust (MSVC) / Node.js / ffmpeg / WebView2

```bash
cd desktop
npm install
npm run tauri dev
```

開発中はPATH上のffmpegを使うため、同梱の準備は不要です。

### テスト

```bash
cd desktop/src-tauri && cargo test
```

LTCの実装はWeb版(TypeScript)とデスクトップ版(Rust)に分かれていますが、**両者の出力が一致することをテストで担保**しています。片方だけ変更するとテストが落ちます。

### 配布用ビルド

```bash
node scripts/package-portable.mjs
```

`dist-portable/TimecodeMix/` に配布用フォルダ一式が出力されます。このフォルダをUSBにコピーして配布してください。インストール不要・管理者権限不要で動作します。

---

## ディレクトリ構成

```text
TimecodeMix/
├── index.html            Web版のUI
├── src/                  Web版のUIロジック
├── core/                 ★ 両版が共有する純粋ロジック
│   ├── ltc.ts            LTC信号の生成、タイムコードの相互変換
│   ├── wav.ts            WAVのエンコード / ヘッダ解析
│   └── video.ts          ffprobe出力の解釈、フレーム量子化
├── desktop/              デスクトップ版 (Tauri)
│   ├── src/              UIロジック
│   └── src-tauri/src/
│       ├── ltc.rs        LTC生成 (core/ltc.ts と同一出力)
│       ├── ffmpeg.rs     ffmpeg / ffprobe の実行
│       └── render.rs     書き出しパイプライン
├── scripts/              セットアップ・配布用スクリプト
├── assets/               アイコンの元データ
└── docs/architecture.md  設計ドキュメント
```

設計の背景や判断理由は [docs/architecture.md](docs/architecture.md) にまとめています。

---

## ライセンス

本プロジェクトは商用・個人利用問わず自由にご利用いただけます。
