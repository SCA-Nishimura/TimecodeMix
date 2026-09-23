/**
 * WAV Encoder Module
 * Supports 16-bit and 24-bit PCM stereo output
 */

/**
 * Encodes Left and Right channels into a WAV file blob.
 * @param leftChannel Float32Array containing L channel samples (LTC)
 * @param rightChannel Float32Array containing R channel samples (Audio)
 * @param sampleRate The target sample rate
 * @param bitDepth The target bit depth (16 or 24)
 */
export function encodeWav(
  leftChannel: Float32Array,
  rightChannel: Float32Array,
  sampleRate: number,
  bitDepth: 16 | 24
): Blob {
  const numChannels = 2;
  const numSamples = leftChannel.length;
  const bytesPerSample = bitDepth / 8;
  const blockAlign = numChannels * bytesPerSample;
  const byteRate = sampleRate * blockAlign;
  const dataSize = numSamples * blockAlign;
  const headerSize = 44;
  const totalSize = headerSize + dataSize;

  const arrayBuffer = new ArrayBuffer(totalSize);
  const view = new DataView(arrayBuffer);

  // Helper to write ASCII strings
  const writeString = (offset: number, str: string) => {
    for (let i = 0; i < str.length; i++) {
      view.setUint8(offset + i, str.charCodeAt(i));
    }
  };

  // 1. RIFF Header
  writeString(0, 'RIFF');
  view.setUint32(4, totalSize - 8, true);
  writeString(8, 'WAVE');

  // 2. fmt Subchunk
  writeString(12, 'fmt ');
  view.setUint32(16, 16, true); // Subchunk size (16 for PCM)
  view.setUint16(20, 1, true);   // Audio format (1 for PCM / Integer)
  view.setUint16(22, numChannels, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, byteRate, true);
  view.setUint16(32, blockAlign, true);
  view.setUint16(34, bitDepth, true);

  // 3. data Subchunk
  writeString(36, 'data');
  view.setUint32(40, dataSize, true);

  // 4. Write PCM Audio samples (interleaved)
  let offset = 44;

  if (bitDepth === 16) {
    for (let i = 0; i < numSamples; i++) {
      // L channel (LTC)
      let sampleL = Math.max(-1, Math.min(1, leftChannel[i]));
      let valL = sampleL < 0 ? sampleL * 0x8000 : sampleL * 0x7FFF;
      view.setInt16(offset, valL, true);
      offset += 2;

      // R channel (Original Audio mixed to mono)
      let sampleR = Math.max(-1, Math.min(1, rightChannel[i]));
      let valR = sampleR < 0 ? sampleR * 0x8000 : sampleR * 0x7FFF;
      view.setInt16(offset, valR, true);
      offset += 2;
    }
  } else if (bitDepth === 24) {
    for (let i = 0; i < numSamples; i++) {
      // L channel (LTC)
      let sampleL = Math.max(-1, Math.min(1, leftChannel[i]));
      let valL = sampleL < 0 ? sampleL * 0x800000 : sampleL * 0x7FFFFF;
      let intL = Math.floor(valL);
      view.setUint8(offset, intL & 0xFF);
      view.setUint8(offset + 1, (intL >> 8) & 0xFF);
      view.setUint8(offset + 2, (intL >> 16) & 0xFF);
      offset += 3;

      // R channel (Original Audio mixed to mono)
      let sampleR = Math.max(-1, Math.min(1, rightChannel[i]));
      let valR = sampleR < 0 ? sampleR * 0x800000 : sampleR * 0x7FFFFF;
      let intR = Math.floor(valR);
      view.setUint8(offset, intR & 0xFF);
      view.setUint8(offset + 1, (intR >> 8) & 0xFF);
      view.setUint8(offset + 2, (intR >> 16) & 0xFF);
      offset += 3;
    }
  }

  return new Blob([arrayBuffer], { type: 'audio/wav' });
}

/**
 * Simple WAV header parser to extract sample rate and bit depth
 */
export interface WavInfo {
  sampleRate: number;
  bitsPerSample: number;
  numChannels: number;
  audioFormat: number;
}

export function parseWavInfo(arrayBuffer: ArrayBuffer): WavInfo | null {
  const view = new DataView(arrayBuffer);
  
  if (arrayBuffer.byteLength < 44) return null;
  
  const riff = String.fromCharCode(view.getUint8(0), view.getUint8(1), view.getUint8(2), view.getUint8(3));
  const wave = String.fromCharCode(view.getUint8(8), view.getUint8(9), view.getUint8(10), view.getUint8(11));
  
  if (riff !== 'RIFF' || wave !== 'WAVE') {
    return null; 
  }

  let offset = 12;
  while (offset < arrayBuffer.byteLength - 8) {
    const chunkId = String.fromCharCode(
      view.getUint8(offset),
      view.getUint8(offset + 1),
      view.getUint8(offset + 2),
      view.getUint8(offset + 3)
    );
    const chunkSize = view.getUint32(offset + 4, true);
    
    if (chunkId === 'fmt ') {
      if (chunkSize < 16) return null;
      const audioFormat = view.getUint16(offset + 8, true);
      const numChannels = view.getUint16(offset + 10, true);
      const sampleRate = view.getUint32(offset + 12, true);
      const bitsPerSample = view.getUint16(offset + 22, true);
      return { sampleRate, bitsPerSample, numChannels, audioFormat };
    }
    
    // Jump to next chunk
    offset += 8 + chunkSize;
  }
  
  return null;
}
