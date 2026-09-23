import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open, save } from '@tauri-apps/plugin-dialog';
import { FRAME_RATES, generateLtcBuffer, parseTimecode, frameToTimecode, formatTimecode } from '../../core/ltc.ts';
import { detectFromProbe, quantizeToFrames, type VideoDetection } from '../../core/video.ts';

const el = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const envStatus = el('env-status');
const dropzone = el('dropzone');
const btnSelect = el<HTMLButtonElement>('btn-select');
const analysis = el('analysis');
const meta = el('meta');
const recommendation = el('recommendation');
const warnings = el('warnings');
const settings = el('settings');
const fpsSelect = el<HTMLSelectElement>('fps-select');
const tcStart = el<HTMLInputElement>('tc-start');
const prerollInput = el<HTMLInputElement>('preroll');
const postrollInput = el<HTMLInputElement>('postroll');
const ltcLevelSelect = el<HTMLSelectElement>('ltc-level');
const bitDepthSelect = el<HTMLSelectElement>('bit-depth');
const paddingNote = el('padding-note');
const btnRender = el<HTMLButtonElement>('btn-render');
const progress = el('progress');

const VIDEO_EXTENSIONS = ['mp4', 'mov', 'm4v'];
/** 黒フレームを本編と同じ形式で作れるコーデック。これ以外は前後の付加ができない */
const PADDABLE_CODECS = ['h264', 'hevc', 'prores'];
/** LTCを一括生成すると長尺でメモリが尽きるため、この秒数ずつ生成して書き出す */
const CHUNK_SECONDS = 30;

interface LoadedVideo {
  path: string;
  detection: VideoDetection;
  durationSeconds: number;
  width: number;
  height: number;
  pixelFormat: string;
  videoCodec: string;
  fpsNum: number;
  fpsDen: number;
  sampleRate: number;
  sourceChannels: number;
}

let loaded: LoadedVideo | null = null;

const isTauri = '__TAURI_INTERNALS__' in window;

async function init() {
  buildSelectOptions();

  if (!isTauri) {
    envStatus.textContent =
      'ブラウザで開かれています。このアプリはデスクトップウィンドウで実行してください。';
    envStatus.classList.add('is-error');
    dropzone.classList.add('is-disabled');
    return;
  }

  try {
    envStatus.textContent = await invoke<string>('check_ffmpeg');
    envStatus.classList.add('is-ok');
  } catch (e) {
    envStatus.textContent = String(e);
    envStatus.classList.add('is-error');
    dropzone.classList.add('is-disabled');
    return;
  }

  btnSelect.addEventListener('click', selectFile);
  btnRender.addEventListener('click', render);
  for (const input of [prerollInput, postrollInput, fpsSelect]) {
    input.addEventListener('change', updatePaddingNote);
  }

  // HTML5のdropではパスが取れないため、Tauri側のイベントを使う
  await getCurrentWebview().onDragDropEvent(event => {
    if (event.payload.type === 'over') {
      dropzone.classList.add('is-dragover');
    } else if (event.payload.type === 'drop') {
      dropzone.classList.remove('is-dragover');
      const path = event.payload.paths[0];
      if (path) void loadFile(path);
    } else {
      dropzone.classList.remove('is-dragover');
    }
  });
}

function buildSelectOptions() {
  fpsSelect.innerHTML = FRAME_RATES.map(
    f => `<option value="${f.id}">${f.label}</option>`
  ).join('');

  const levels: string[] = [];
  for (let db = 0; db >= -18; db--) {
    levels.push(`<option value="${db}"${db === -6 ? ' selected' : ''}>${db} dBFS</option>`);
  }
  ltcLevelSelect.innerHTML = levels.join('');
}

async function selectFile() {
  const selected = await open({
    multiple: false,
    filters: [{ name: '映像ファイル', extensions: VIDEO_EXTENSIONS }],
  });
  if (typeof selected === 'string') {
    await loadFile(selected);
  }
}

async function loadFile(path: string) {
  const extension = path.split('.').pop()?.toLowerCase() ?? '';
  if (!VIDEO_EXTENSIONS.includes(extension)) {
    showError(`対応していない形式です (.${extension})。MP4 または MOV を指定してください。`);
    return;
  }

  try {
    const probe = JSON.parse(await invoke<string>('probe_video', { path }));
    const video = (probe.streams ?? []).find((s: any) => s.codec_type === 'video');
    const audio = (probe.streams ?? []).find((s: any) => s.codec_type === 'audio');
    if (!video) {
      showError('映像トラックが見つかりませんでした。');
      return;
    }

    const [fpsNum, fpsDen] = String(video.r_frame_rate).split('/').map(Number);
    loaded = {
      path,
      detection: detectFromProbe(probe),
      durationSeconds: Number(probe.format?.duration ?? 0),
      width: Number(video.width),
      height: Number(video.height),
      pixelFormat: video.pix_fmt ?? 'yuv420p',
      videoCodec: video.codec_name ?? '',
      fpsNum: fpsNum || 30000,
      fpsDen: fpsDen || 1001,
      sampleRate: Number(audio?.sample_rate ?? 48000),
      sourceChannels: Number(audio?.channels ?? 0),
    };

    renderAnalysis(loaded, video, audio);
    applyDetectionDefaults(loaded.detection);
    settings.classList.remove('hidden');
    updatePaddingNote();
    progress.textContent = '';
  } catch (e) {
    showError(String(e));
  }
}

function renderAnalysis(video: LoadedVideo, videoStream: any, audioStream: any) {
  const rows: [string, string][] = [
    ['ファイル', video.path.split(/[\\/]/).pop() ?? video.path],
    ['長さ', formatDuration(video.durationSeconds)],
    ['映像', `${video.videoCodec} / ${video.width}×${video.height} / ${video.detection.detectedFpsLabel}fps`],
    [
      '音声',
      audioStream
        ? `${audioStream.codec_name} / ${video.sampleRate.toLocaleString()} Hz / ${video.sourceChannels}ch`
        : 'なし(無音で出力されます)',
    ],
    ['元素材の開始TC', video.detection.embeddedTimecode ?? '記録なし'],
  ];
  void videoStream;

  meta.innerHTML = rows
    .map(([key, value]) => `<dt>${escapeHtml(key)}</dt><dd>${escapeHtml(value)}</dd>`)
    .join('');
  recommendation.textContent = video.detection.recommendation;
  warnings.innerHTML = video.detection.warnings
    .map(w => `<p class="warning">${escapeHtml(w)}</p>`)
    .join('');
  analysis.classList.remove('hidden');
}

function applyDetectionDefaults(detection: VideoDetection) {
  if (detection.fpsId) {
    fpsSelect.value = detection.fpsId;
  }
  if (detection.embeddedTimecode) {
    // tmcdはドロップフレームを ';' で表すが、入力欄は ':' 区切りに揃える
    tcStart.value = detection.embeddedTimecode.replace(';', ':');
  }
}

function currentConfig() {
  return FRAME_RATES.find(f => f.id === fpsSelect.value) ?? FRAME_RATES[0];
}

function updatePaddingNote() {
  if (!loaded) return;
  const padded =
    parseFloat(prerollInput.value || '0') > 0 || parseFloat(postrollInput.value || '0') > 0;
  const canPad = PADDABLE_CODECS.includes(loaded.videoCodec);

  if (padded && !canPad) {
    paddingNote.textContent =
      `このファイルのコーデック(${loaded.videoCodec})では、本編と同じ形式の黒フレームを作れないため` +
      'プリロール/ポストロールを付けられません。どちらも0にしてください。';
    paddingNote.className = 'note is-error';
    btnRender.disabled = true;
    return;
  }

  paddingNote.textContent = padded
    ? '前後に付ける黒の部分だけをエンコードし、本編の映像は再エンコードせずそのまま引き継ぎます。'
    : '映像は再エンコードせずそのまま引き継ぎます。';
  paddingNote.className = 'note';
  btnRender.disabled = false;
}

/** 秒指定のプリ/ポストロールを整数フレームに丸め、尺と総サンプル数を確定させる */
function computeTiming(video: LoadedVideo) {
  const actualFps = video.fpsNum / video.fpsDen;
  const preroll = quantizeToFrames(Math.max(0, parseFloat(prerollInput.value || '0')), actualFps);
  const postroll = quantizeToFrames(Math.max(0, parseFloat(postrollInput.value || '0')), actualFps);
  const sourceFrames = Math.round(video.durationSeconds * actualFps);
  const totalFrames = preroll.frames + sourceFrames + postroll.frames;

  return {
    actualFps,
    prerollFrames: preroll.frames,
    postrollFrames: postroll.frames,
    totalFrames,
    totalSamples: Math.round((totalFrames / actualFps) * video.sampleRate),
  };
}

async function render() {
  if (!loaded) return;

  const config = currentConfig();
  let startFrames: number;
  try {
    startFrames = parseTimecode(tcStart.value, config);
  } catch (e: any) {
    showProgress(e.message ?? '開始タイムコードの形式が正しくありません。', true);
    return;
  }

  const defaultName = (loaded.path.split(/[\\/]/).pop() ?? 'output').replace(/\.[^.]+$/, '');
  const outputPath = await save({
    defaultPath: `${defaultName}_ltc_${config.id}.mov`,
    filters: [{ name: 'QuickTime Movie', extensions: ['mov'] }],
  });
  if (!outputPath) return;

  btnRender.disabled = true;
  const timing = computeTiming(loaded);

  try {
    showProgress('タイムコードを生成しています...');
    const ltcPath = await invoke<string>('create_ltc_file');
    await writeLtc(ltcPath, timing, config, startFrames, loaded.sampleRate);

    showProgress('映像を書き出しています...');
    await invoke<string>('render_video', {
      options: {
        inputPath: loaded.path,
        outputPath,
        ltcPath,
        width: loaded.width,
        height: loaded.height,
        pixelFormat: loaded.pixelFormat,
        videoCodec: loaded.videoCodec,
        fpsNum: loaded.fpsNum,
        fpsDen: loaded.fpsDen,
        sampleRate: loaded.sampleRate,
        bitDepth: Number(bitDepthSelect.value),
        sourceChannels: loaded.sourceChannels,
        prerollFrames: timing.prerollFrames,
        postrollFrames: timing.postrollFrames,
        totalSamples: timing.totalSamples,
        startTimecode: formatTimecode(
          frameToTimecode(startFrames, config),
          config.isDropFrame
        ),
      },
    });

    showProgress(`書き出しました: ${outputPath}`);
  } catch (e) {
    showProgress(String(e), true);
  } finally {
    btnRender.disabled = false;
  }
}

/**
 * LTCを一定時間ずつ生成してRust側へ追記していく。
 * 一括生成すると長尺でFloat32Arrayがメモリを食い尽くすため。
 */
async function writeLtc(
  ltcPath: string,
  timing: ReturnType<typeof computeTiming>,
  config: (typeof FRAME_RATES)[number],
  startFrames: number,
  sampleRate: number
) {
  const framesPerChunk = Math.max(1, Math.round(CHUNK_SECONDS * timing.actualFps));
  let written = 0;

  while (written < timing.totalFrames) {
    const frames = Math.min(framesPerChunk, timing.totalFrames - written);
    const buffer = generateLtcBuffer(
      frames / timing.actualFps,
      sampleRate,
      startFrames + written,
      Number(ltcLevelSelect.value),
      config
    );

    await invoke('append_ltc', {
      path: ltcPath,
      chunk: toBase64(new Uint8Array(buffer.buffer, 0, buffer.length * 4)),
    });

    written += frames;
    showProgress(
      `タイムコードを生成しています... ${Math.round((written / timing.totalFrames) * 100)}%`
    );
    // 進捗表示を描画させる
    await new Promise(resolve => requestAnimationFrame(resolve));
  }
}

function toBase64(bytes: Uint8Array): string {
  let binary = '';
  const STEP = 0x8000;
  for (let i = 0; i < bytes.length; i += STEP) {
    binary += String.fromCharCode(...bytes.subarray(i, i + STEP));
  }
  return btoa(binary);
}

function showProgress(message: string, isError = false) {
  progress.textContent = message;
  progress.className = isError ? 'progress is-error' : 'progress';
}

function showError(message: string) {
  meta.innerHTML = '';
  recommendation.textContent = '';
  warnings.innerHTML = `<p class="warning">${escapeHtml(message)}</p>`;
  analysis.classList.remove('hidden');
  settings.classList.add('hidden');
}

function formatDuration(seconds: number): string {
  if (!seconds) return '不明';
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  const pad = (n: number) => n.toString().padStart(2, '0');
  return `${pad(h)}:${pad(m)}:${s.toFixed(2).padStart(5, '0')}`;
}

function escapeHtml(value: string): string {
  const div = document.createElement('div');
  div.textContent = value;
  return div.innerHTML;
}

void init();
