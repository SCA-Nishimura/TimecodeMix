/**
 * LTC (Linear Timecode) Generator Module
 * Standard: SMPTE 12M
 * Supported Frame rates: Selectable (23.976, 24, 25, 29.97 DF/NDF, 30 DF/NDF)
 * Amplitude: Variable, -18 to 0 dBFS (default -6 dBFS)
 */

export interface FrameRateConfig {
  id: string;
  label: string;
  timecodeFps: number;
  actualFps: number;
  isDropFrame: boolean;
}

export const FRAME_RATES: FrameRateConfig[] = [
  { id: '30-ndf', label: '30FPS(NDF)', timecodeFps: 30, actualFps: 30, isDropFrame: false },
  { id: '30-df', label: '30FPS(DF)', timecodeFps: 30, actualFps: 30000 / 1001, isDropFrame: true },
  { id: '29.97-ndf', label: '29.97FPS(NDF)', timecodeFps: 30, actualFps: 30000 / 1001, isDropFrame: false },
  { id: '29.97-df', label: '29.97FPS(DF)', timecodeFps: 30, actualFps: 30000 / 1001, isDropFrame: true },
  { id: '25', label: '25FPS', timecodeFps: 25, actualFps: 25, isDropFrame: false },
  { id: '24', label: '24FPS', timecodeFps: 24, actualFps: 24, isDropFrame: false },
  { id: '23.976', label: '23.976FPS', timecodeFps: 24, actualFps: 24000 / 1001, isDropFrame: false },
];

export interface Timecode {
  hours: number;
  minutes: number;
  seconds: number;
  frames: number;
}

/**
 * Converts a total frame index into a Timecode object based on frame rate config
 */
export function frameToTimecode(totalFrames: number, config: FrameRateConfig): Timecode {
  if (config.isDropFrame) {
    let f = totalFrames;
    const d = Math.floor(f / 17982);
    const m = f % 17982;
    
    let minutes = d * 10;
    let frames_remaining = m;
    if (frames_remaining >= 1800) {
      frames_remaining -= 1800;
      const extra_minutes = Math.floor(frames_remaining / 1798) + 1;
      minutes += extra_minutes;
      frames_remaining = frames_remaining % 1798;
      frames_remaining += 2;
    }
    const seconds = Math.floor(frames_remaining / 30);
    const frames = frames_remaining % 30;
    const hours = Math.floor(minutes / 60) % 24;
    minutes = minutes % 60;
    
    return { hours, minutes, seconds, frames };
  } else {
    let f = totalFrames;
    const fps = config.timecodeFps;
    
    const frames = f % fps;
    f = Math.floor(f / fps);
    
    const seconds = f % 60;
    f = Math.floor(f / 60);
    
    const minutes = f % 60;
    f = Math.floor(f / 60);
    
    const hours = f % 24;
    
    return { hours, minutes, seconds, frames };
  }
}

/**
 * Format timecode object as HH:MM:SS:FF (or HH:MM:SS;FF for drop frame) string
 */
export function formatTimecode(tc: Timecode, isDropFrame: boolean): string {
  const pad = (n: number) => n.toString().padStart(2, '0');
  const separator = isDropFrame ? ';' : ':';
  return `${pad(tc.hours)}:${pad(tc.minutes)}:${pad(tc.seconds)}${separator}${pad(tc.frames)}`;
}

/**
 * Generates the 80 bits for a specific frame index
 */
export function generateLtcBits(totalFrameIndex: number, config: FrameRateConfig): number[] {
  const tc = frameToTimecode(totalFrameIndex, config);
  const bits = new Array<number>(80).fill(0);

  const f_units = tc.frames % 10;
  const f_tens = Math.floor(tc.frames / 10);
  
  const s_units = tc.seconds % 10;
  const s_tens = Math.floor(tc.seconds / 10);
  
  const m_units = tc.minutes % 10;
  const m_tens = Math.floor(tc.minutes / 10);
  
  const h_units = tc.hours % 10;
  const h_tens = Math.floor(tc.hours / 10);

  // Helper to write number as BCD (Binary Coded Decimal) into specific bits
  const writeBcd = (val: number, startBit: number, numBits: number) => {
    for (let i = 0; i < numBits; i++) {
      bits[startBit + i] = (val >> i) & 1;
    }
  };

  // 1. Write Timecode data
  // Frame units (0-3)
  writeBcd(f_units, 0, 4);
  // Frame tens (8-9)
  writeBcd(f_tens, 8, 2);
  // Bit 10 is drop frame flag (1 for drop, 0 for non-drop)
  bits[10] = config.isDropFrame ? 1 : 0;
  // Bit 11 is color frame flag (0)
  bits[11] = 0;

  // Second units (16-19)
  writeBcd(s_units, 16, 4);
  // Second tens (24-26)
  writeBcd(s_tens, 24, 3);
  // Bit 27: Bi-phase mark correction (set later)

  // Minute units (32-35)
  writeBcd(m_units, 32, 4);
  // Minute tens (40-42)
  writeBcd(m_tens, 40, 3);
  // Bit 43: BGF0 (0)
  bits[43] = 0;

  // Hour units (48-51)
  writeBcd(h_units, 48, 4);
  // Hour tens (56-57)
  writeBcd(h_tens, 56, 2);
  // Bit 58: BGF1 (0)
  bits[58] = 0;
  // Bit 59: BGF2 (0)
  bits[59] = 0;

  // User bits (remain 0)
  // User Group 1 (4-7), 2 (12-15), 3 (20-23), 4 (28-31), 5 (36-39), 6 (44-47), 7 (52-55), 8 (60-63)

  // 2. Set Sync Word (64-79)
  // Pattern: 0011 1111 1111 1101
  bits[64] = 0;
  bits[65] = 0;
  for (let i = 66; i <= 77; i++) {
    bits[i] = 1;
  }
  bits[78] = 0;
  bits[79] = 1;

  // 3. Parity calculation for Bi-phase mark correction (Bit 27)
  // The parity bit ensures that the total number of transitions is even,
  // meaning there is an even number of '1' bits in the entire frame of 80 bits.
  let oneCount = 0;
  for (let i = 0; i < 80; i++) {
    if (i !== 27 && bits[i] === 1) {
      oneCount++;
    }
  }
  // If oneCount is odd, bits[27] = 1 to make the total count of 1s even.
  bits[27] = (oneCount % 2 === 1) ? 1 : 0;

  return bits;
}

/**
 * Generates Float32Array containing LTC audio signal at specified level and sample rate
 */
export function generateLtcBuffer(
  durationSeconds: number,
  sampleRate: number,
  startFramesOffset: number,
  levelDbfs: number,
  config: FrameRateConfig
): Float32Array {
  const totalSamples = Math.floor(durationSeconds * sampleRate);
  const totalFrames = Math.ceil(durationSeconds * config.actualFps);
  const ltcBuffer = new Float32Array(totalSamples);
  
  // Calculate amplitude: levelDbfs dBFS -> 10^(levelDbfs / 20)
  const amplitude = Math.pow(10, levelDbfs / 20);
  
  let currentLevel = -1.0;

  for (let f = 0; f < totalFrames; f++) {
    const frameStart = Math.floor(f * sampleRate / config.actualFps);
    const frameEnd = Math.floor((f + 1) * sampleRate / config.actualFps);
    const actualSamplesInFrame = frameEnd - frameStart;
    
    // Get the bits for this frame
    const frameBits = generateLtcBits(f + startFramesOffset, config);

    for (let b = 0; b < 80; b++) {
      const bitStart = frameStart + Math.floor(b * actualSamplesInFrame / 80);
      const bitEnd = frameStart + Math.min(Math.floor((b + 1) * actualSamplesInFrame / 80), actualSamplesInFrame);
      const bitMid = frameStart + Math.floor((b + 0.5) * actualSamplesInFrame / 80);
      
      const bitValue = frameBits[b];

      // BMC transitions:
      // Transition at start of bit
      currentLevel = -currentLevel;

      if (bitValue === 1) {
        // First half of the bit
        const endFirstHalf = Math.min(bitMid, totalSamples);
        for (let i = bitStart; i < endFirstHalf; i++) {
          if (i < totalSamples) ltcBuffer[i] = currentLevel * amplitude;
        }
        
        // Transition at middle of bit
        currentLevel = -currentLevel;
        
        // Second half of the bit
        const endSecondHalf = Math.min(bitEnd, totalSamples);
        for (let i = bitMid; i < endSecondHalf; i++) {
          if (i < totalSamples) ltcBuffer[i] = currentLevel * amplitude;
        }
      } else {
        // Bit value 0: Level remains constant throughout the bit period
        const endBit = Math.min(bitEnd, totalSamples);
        for (let i = bitStart; i < endBit; i++) {
          if (i < totalSamples) ltcBuffer[i] = currentLevel * amplitude;
        }
      }
    }
  }

  return ltcBuffer;
}
