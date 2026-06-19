import { generateLtcBuffer } from './ltc.ts';
import { encodeWav, parseWavInfo } from './wav.ts';

// DOM Elements
const dropzone = document.getElementById('dropzone') as HTMLDivElement;
const fileInput = document.getElementById('file-input') as HTMLInputElement;
const processPanel = document.getElementById('process-panel') as HTMLDivElement;
const resultPanel = document.getElementById('result-panel') as HTMLDivElement;

const metaName = document.getElementById('meta-name') as HTMLSpanElement;
const metaFormat = document.getElementById('meta-format') as HTMLSpanElement;
const metaSampleRate = document.getElementById('meta-samplerate') as HTMLSpanElement;
const metaBitDepth = document.getElementById('meta-bitdepth') as HTMLSpanElement;
const metaDuration = document.getElementById('meta-duration') as HTMLSpanElement;

const tcStartInput = document.getElementById('tc-start') as HTMLInputElement;
const silenceDelayInput = document.getElementById('silence-delay') as HTMLInputElement;
const outBitDepthSelect = document.getElementById('out-bitdepth') as HTMLSelectElement;

const btnProcess = document.getElementById('btn-process') as HTMLButtonElement;
const btnProcessText = btnProcess.querySelector('.btn-text') as HTMLSpanElement;
const btnProcessSpinner = btnProcess.querySelector('.btn-spinner') as HTMLSpanElement;

const btnPlay = document.getElementById('btn-play') as HTMLButtonElement;
const playerTimeCurrent = document.getElementById('player-time-current') as HTMLSpanElement;
const playerTimeTotal = document.getElementById('player-time-total') as HTMLSpanElement;
const playerProgressBar = document.getElementById('player-progress-bar') as HTMLDivElement;
const playerProgressFill = document.getElementById('player-progress-fill') as HTMLDivElement;
const chkMuteLtc = document.getElementById('chk-mute-ltc') as HTMLInputElement;

const btnDownload = document.getElementById('btn-download') as HTMLAnchorElement;

// State Variables
let loadedFile: File | null = null;
let loadedArrayBuffer: ArrayBuffer | null = null;
let decodedAudioBuffer: AudioBuffer | null = null;
let detectedBitDepth: 16 | 24 | null = null;

// Web Audio API playback state
let audioCtx: AudioContext | null = null;
let sourceNode: AudioBufferSourceNode | null = null;
let gainNodeL: GainNode | null = null;
let isPlaying = false;
let startTime = 0;
let pauseOffset = 0;
let previewBuffer: AudioBuffer | null = null;
let animationFrameId = 0;
let generatedWavUrl: string | null = null;

// Setup Event Listeners
initEvents();

function initEvents() {
  // Drag and Drop
  ['dragenter', 'dragover'].forEach(eventName => {
    dropzone.addEventListener(eventName, (e) => {
      e.preventDefault();
      dropzone.classList.add('dragover');
    }, false);
  });

  ['dragleave', 'drop'].forEach(eventName => {
    dropzone.addEventListener(eventName, (e) => {
      e.preventDefault();
      dropzone.classList.remove('dragover');
    }, false);
  });

  dropzone.addEventListener('drop', (e) => {
    const dt = e.dataTransfer;
    if (dt && dt.files.length > 0) {
      handleFile(dt.files[0]);
    }
  });

  fileInput.addEventListener('change', () => {
    if (fileInput.files && fileInput.files.length > 0) {
      handleFile(fileInput.files[0]);
    }
  });

  // Processing Click
  btnProcess.addEventListener('click', processAudio);

  // Playback controls
  btnPlay.addEventListener('click', togglePlayback);
  chkMuteLtc.addEventListener('change', () => {
    if (gainNodeL) {
      gainNodeL.gain.value = chkMuteLtc.checked ? 0 : 1;
    }
  });

  playerProgressBar.addEventListener('click', seekPlayback);
}

/**
 * Handle the selected/dropped file
 */
async function handleFile(file: File) {
  loadedFile = file;
  resetState();

  metaName.textContent = file.name;
  metaName.title = file.name;

  try {
    loadedArrayBuffer = await file.arrayBuffer();
    
    // Parse binary header if WAV
    const wavInfo = parseWavInfo(loadedArrayBuffer);
    if (wavInfo) {
      metaFormat.textContent = 'WAV';
      metaSampleRate.textContent = `${wavInfo.sampleRate.toLocaleString()} Hz`;
      detectedBitDepth = wavInfo.bitsPerSample === 24 ? 24 : 16;
      metaBitDepth.textContent = `${wavInfo.bitsPerSample}-bit`;
    } else {
      metaFormat.textContent = file.type.includes('wav') ? 'WAV' : 'MP3/Compressed';
      metaBitDepth.textContent = 'N/A (Compressed)';
      detectedBitDepth = null;
    }

    // Decode Audio Data
    if (!audioCtx) {
      audioCtx = new (window.AudioContext || (window as any).webkitAudioContext)();
    }
    
    // Show temporary decoding status
    metaDuration.textContent = 'Decoding...';
    
    decodedAudioBuffer = await audioCtx.decodeAudioData(loadedArrayBuffer.slice(0));
    
    if (!wavInfo) {
      metaSampleRate.textContent = `${decodedAudioBuffer.sampleRate.toLocaleString()} Hz`;
    }

    const durationStr = formatSeconds(decodedAudioBuffer.duration);
    metaDuration.textContent = durationStr;

    // Show setting panel
    processPanel.classList.remove('hidden');
    resultPanel.classList.add('hidden');
    
    // Scroll to the config card
    processPanel.scrollIntoView({ behavior: 'smooth' });

  } catch (error) {
    console.error('File parsing failed:', error);
    alert('Failed to parse audio file. Please verify it is a valid MP3 or WAV file.');
    resetState();
  }
}

/**
 * Parses timecode string (HH:MM:SS:FF or H:M:S:F) into total frame count
 */
function parseTimecode(str: string): number {
  const parts = str.trim().split(/[:.]/);
  if (parts.length !== 4) {
    throw new Error('Invalid format. Use HH:MM:SS:FF (e.g. 01:00:00:00)');
  }
  const h = parseInt(parts[0], 10);
  const m = parseInt(parts[1], 10);
  const s = parseInt(parts[2], 10);
  const f = parseInt(parts[3], 10);

  if (isNaN(h) || isNaN(m) || isNaN(s) || isNaN(f)) {
    throw new Error('Timecode contains non-numeric values.');
  }

  if (h < 0 || m < 0 || m >= 60 || s < 0 || s >= 60 || f < 0 || f >= 30) {
    throw new Error('Values out of range. Frame must be 0-29. Min/Sec must be 0-59.');
  }

  // Convert to total frames at 30 fps
  return (h * 3600 + m * 60 + s) * 30 + f;
}

/**
 * Mix the LTC track and output the WAV file
 */
async function processAudio() {
  if (!decodedAudioBuffer) return;

  // 1. Get and Validate Settings
  let startFramesOffset = 108000; // 01:00:00:00 (1:0:0:0) default
  try {
    startFramesOffset = parseTimecode(tcStartInput.value);
  } catch (e: any) {
    alert(e.message || 'Invalid start timecode format. Please use HH:MM:SS:FF (e.g. 01:00:00:00)');
    tcStartInput.focus();
    return;
  }

  let silenceSeconds = 0;
  const silenceVal = parseFloat(silenceDelayInput.value);
  if (!isNaN(silenceVal) && silenceVal >= 0) {
    silenceSeconds = silenceVal;
  } else {
    alert('Please enter a valid non-negative number for Silence Delay.');
    silenceDelayInput.focus();
    return;
  }

  // 2. Set UI processing state
  btnProcess.disabled = true;
  btnProcessText.textContent = 'Processing...';
  btnProcessSpinner.classList.remove('hidden');

  // Let UI update
  await new Promise(resolve => requestAnimationFrame(resolve));

  try {
    const sampleRate = decodedAudioBuffer.sampleRate;
    const audioSamples = decodedAudioBuffer.length;
    const silenceSamples = Math.floor(silenceSeconds * sampleRate);
    const totalSamples = silenceSamples + audioSamples;
    const totalDuration = totalSamples / sampleRate;

    // 3. Generate LTC Buffer for L Channel
    const ltcL = generateLtcBuffer(totalDuration, sampleRate, startFramesOffset, -6);

    // 4. Downmix Input Audio to Mono for R Channel with Silence padding
    const audioR = new Float32Array(totalSamples);
    if (decodedAudioBuffer.numberOfChannels >= 2) {
      const leftIn = decodedAudioBuffer.getChannelData(0);
      const rightIn = decodedAudioBuffer.getChannelData(1);
      for (let i = 0; i < audioSamples; i++) {
        audioR[silenceSamples + i] = (leftIn[i] + rightIn[i]) / 2;
      }
    } else {
      const channelData = decodedAudioBuffer.getChannelData(0);
      audioR.set(channelData, silenceSamples);
    }

    // 5. Determine Export Bit Depth
    let targetBitDepth: 16 | 24 = 16;
    const selection = outBitDepthSelect.value;
    if (selection === 'auto') {
      targetBitDepth = detectedBitDepth || 16;
    } else {
      targetBitDepth = selection === '24' ? 24 : 16;
    }

    // 6. Encode WAV
    const wavBlob = encodeWav(ltcL, audioR, sampleRate, targetBitDepth);

    // 7. Cleanup old object URL and create new one
    if (generatedWavUrl) {
      URL.revokeObjectURL(generatedWavUrl);
    }
    generatedWavUrl = URL.createObjectURL(wavBlob);
    
    // Set download link
    btnDownload.href = generatedWavUrl;
    const originalBase = loadedFile ? loadedFile.name.replace(/\.[^/.]+$/, "") : "audio";
    btnDownload.download = `${originalBase}_ltc_30fps.wav`;

    // 8. Prepare Preview AudioBuffer
    if (audioCtx) {
      previewBuffer = audioCtx.createBuffer(2, totalSamples, sampleRate);
      previewBuffer.getChannelData(0).set(ltcL);
      previewBuffer.getChannelData(1).set(audioR);
    }

    // Setup preview time display
    playerTimeCurrent.textContent = '00:00';
    playerTimeTotal.textContent = formatSeconds(totalDuration);
    updateProgressFill(0);
    pauseOffset = 0;

    // Show results
    resultPanel.classList.remove('hidden');
    resultPanel.scrollIntoView({ behavior: 'smooth' });

  } catch (error) {
    console.error('Audio processing error:', error);
    alert('An error occurred during audio processing.');
  } finally {
    // Reset UI processing state
    btnProcess.disabled = false;
    btnProcessText.textContent = 'Mix & Generate WAV';
    btnProcessSpinner.classList.add('hidden');
  }
}

/**
 * Handle Play/Pause of the generated track
 */
function togglePlayback() {
  if (!previewBuffer || !audioCtx) return;

  if (isPlaying) {
    pausePreview();
  } else {
    playPreview();
  }
}

function playPreview() {
  if (!previewBuffer || !audioCtx) return;

  if (audioCtx.state === 'suspended') {
    audioCtx.resume();
  }

  // Create Source Node
  sourceNode = audioCtx.createBufferSource();
  sourceNode.buffer = previewBuffer;

  // Splitter & Merger for individual channel gain (LTC mute)
  const splitter = audioCtx.createChannelSplitter(2);
  const merger = audioCtx.createChannelMerger(2);
  gainNodeL = audioCtx.createGain();

  // Set gain for L channel (LTC) based on toggle
  gainNodeL.gain.value = chkMuteLtc.checked ? 0 : 1;

  // Connections
  sourceNode.connect(splitter);
  splitter.connect(gainNodeL, 0); // L channel through gain node
  gainNodeL.connect(merger, 0, 0); // merged L channel
  splitter.connect(merger, 1, 1); // R channel directly connected
  
  merger.connect(audioCtx.destination);

  // Start playback
  const offset = pauseOffset % previewBuffer.duration;
  sourceNode.start(0, offset);
  startTime = audioCtx.currentTime - offset;
  isPlaying = true;
  updatePlayButtonUI();
  
  // Animation frame ticks
  tick();

  sourceNode.onended = () => {
    // Verify if it ended naturally or was stopped manually
    if (isPlaying && audioCtx && (audioCtx.currentTime - startTime) >= previewBuffer!.duration - 0.05) {
      stopPreview(true);
    }
  };
}

function pausePreview() {
  if (!isPlaying || !sourceNode || !audioCtx) return;
  sourceNode.stop();
  sourceNode = null;
  pauseOffset = audioCtx.currentTime - startTime;
  isPlaying = false;
  updatePlayButtonUI();
  cancelAnimationFrame(animationFrameId);
}

function stopPreview(reset = false) {
  if (sourceNode) {
    try {
      sourceNode.stop();
    } catch (e) {
      // already stopped
    }
    sourceNode = null;
  }
  isPlaying = false;
  if (reset) {
    pauseOffset = 0;
    updateProgressFill(0);
    playerTimeCurrent.textContent = '00:00';
  }
  updatePlayButtonUI();
  cancelAnimationFrame(animationFrameId);
}

function tick() {
  if (!isPlaying || !audioCtx || !previewBuffer) return;

  const current = audioCtx.currentTime - startTime;
  const duration = previewBuffer.duration;

  if (current <= duration) {
    playerTimeCurrent.textContent = formatSeconds(current);
    updateProgressFill(current / duration);
    animationFrameId = requestAnimationFrame(tick);
  }
}

function seekPlayback(e: MouseEvent) {
  if (!previewBuffer) return;

  const rect = playerProgressBar.getBoundingClientRect();
  const clickX = e.clientX - rect.left;
  const percent = Math.max(0, Math.min(1, clickX / rect.width));
  const seekTime = percent * previewBuffer.duration;

  if (isPlaying) {
    stopPreview(false);
    pauseOffset = seekTime;
    playPreview();
  } else {
    pauseOffset = seekTime;
    updateProgressFill(percent);
    playerTimeCurrent.textContent = formatSeconds(seekTime);
  }
}

function updateProgressFill(ratio: number) {
  playerProgressFill.style.width = `${ratio * 100}%`;
}

function updatePlayButtonUI() {
  if (isPlaying) {
    btnPlay.innerHTML = `
      <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
        <path d="M6 19H10V5H6V19ZM14 5V19H18V5H18Z" />
      </svg>
    `;
  } else {
    btnPlay.innerHTML = `
      <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
        <path d="M8 5V19L19 12L8 5Z" />
      </svg>
    `;
  }
}

/**
 * Format raw seconds as MM:SS display
 */
function formatSeconds(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return `${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
}

/**
 * Reset local app state
 */
function resetState() {
  stopPreview(true);
  
  loadedArrayBuffer = null;
  decodedAudioBuffer = null;
  detectedBitDepth = null;
  previewBuffer = null;

  if (generatedWavUrl) {
    URL.revokeObjectURL(generatedWavUrl);
    generatedWavUrl = null;
  }

  processPanel.classList.add('hidden');
  resultPanel.classList.add('hidden');
}
