import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open } from '@tauri-apps/plugin-dialog';
import { FRAME_RATES } from '../../core/ltc.ts';
import { detectFromProbe } from '../../core/video.ts';

const envStatus = document.getElementById('env-status') as HTMLElement;
const dropzone = document.getElementById('dropzone') as HTMLElement;
const btnSelect = document.getElementById('btn-select') as HTMLButtonElement;
const result = document.getElementById('result') as HTMLElement;
const meta = document.getElementById('meta') as HTMLElement;
const recommendation = document.getElementById('recommendation') as HTMLElement;
const warnings = document.getElementById('warnings') as HTMLElement;

const VIDEO_EXTENSIONS = ['mp4', 'mov', 'm4v'];

/** Tauri のウィンドウ外(ブラウザ)で開かれた場合は機能しない */
const isTauri = '__TAURI_INTERNALS__' in window;

async function init() {
  if (!isTauri) {
    envStatus.textContent =
      'ブラウザで開かれています。このアプリはデスクトップウィンドウで実行してください。';
    envStatus.classList.add('is-error');
    dropzone.classList.add('is-disabled');
    return;
  }

  try {
    const version = await invoke<string>('check_ffmpeg');
    envStatus.textContent = version;
    envStatus.classList.add('is-ok');
  } catch (e) {
    envStatus.textContent = String(e);
    envStatus.classList.add('is-error');
    dropzone.classList.add('is-disabled');
    return;
  }

  btnSelect.addEventListener('click', selectFile);

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
    const probeJson = await invoke<string>('probe_video', { path });
    render(path, JSON.parse(probeJson));
  } catch (e) {
    showError(String(e));
  }
}

function render(path: string, probe: any) {
  const detection = detectFromProbe(probe);
  const video = (probe.streams ?? []).find((s: any) => s.codec_type === 'video');
  const audio = (probe.streams ?? []).find((s: any) => s.codec_type === 'audio');
  const duration = Number(probe.format?.duration ?? 0);

  const rows: [string, string][] = [
    ['ファイル', path.split(/[\\/]/).pop() ?? path],
    ['長さ', formatDuration(duration)],
    ['映像', video ? `${video.codec_name} / ${video.width}×${video.height} / ${detection.detectedFpsLabel}fps` : 'なし'],
    ['音声', audio ? `${audio.codec_name} / ${Number(audio.sample_rate).toLocaleString()} Hz / ${audio.channels}ch` : 'なし'],
    ['推奨するTCのFPS', FRAME_RATES.find(f => f.id === detection.fpsId)?.label ?? '判定不能'],
    ['元素材の開始TC', detection.embeddedTimecode ?? '記録なし'],
  ];

  meta.innerHTML = rows
    .map(([key, value]) => `<dt>${escapeHtml(key)}</dt><dd>${escapeHtml(value)}</dd>`)
    .join('');

  recommendation.textContent = detection.recommendation;
  warnings.innerHTML = detection.warnings
    .map(w => `<p class="warning">${escapeHtml(w)}</p>`)
    .join('');

  result.classList.remove('hidden');
}

function showError(message: string) {
  meta.innerHTML = '';
  recommendation.textContent = '';
  warnings.innerHTML = `<p class="warning">${escapeHtml(message)}</p>`;
  result.classList.remove('hidden');
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
