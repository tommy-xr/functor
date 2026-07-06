// Generate the sample audio the examples use. Everything here is synthesized
// from scratch (no external samples), so the output is original/CC0 and needs no
// third-party attribution — see README "Credits". The files are small and
// committed (like examples/mle-lighting/bumps-normal.png), so the demos run without
// a generate step; this script is how to reproduce/tweak them.
//
// Usage: npm run generate:audio
//
// What it writes (16-bit PCM mono WAV):
//   examples/mle-lighting/gunshot.wav   a short noise-burst one-shot (Audio.play/playAt)
//   examples/mle-lighting/wind-loop.wav a seamless ambient wind bed (non-spatial)
//   examples/mle-lighting/water-loop.wav a seamless fountain/water loop (positioned)
//
// The looping beds use additive synthesis at integer multiples of the loop
// fundamental (1/duration), so the waveform is exactly periodic over the buffer
// and loops with no seam — no crossfade needed.

import { writeFile } from "node:fs/promises";
import path from "node:path";

// Small deterministic PRNG (mulberry32) so regenerating is byte-stable.
function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// Encode Float32 samples in [-1,1] as a 16-bit PCM mono WAV (RIFF).
function encodeWav(samples, sampleRate) {
  const numSamples = samples.length;
  const dataBytes = numSamples * 2;
  const buffer = Buffer.alloc(44 + dataBytes);
  buffer.write("RIFF", 0, "ascii");
  buffer.writeUInt32LE(36 + dataBytes, 4);
  buffer.write("WAVE", 8, "ascii");
  buffer.write("fmt ", 12, "ascii");
  buffer.writeUInt32LE(16, 16); // PCM fmt chunk size
  buffer.writeUInt16LE(1, 20); // audio format: PCM
  buffer.writeUInt16LE(1, 22); // channels: mono
  buffer.writeUInt32LE(sampleRate, 24);
  buffer.writeUInt32LE(sampleRate * 2, 28); // byte rate (mono, 2 bytes/sample)
  buffer.writeUInt16LE(2, 32); // block align
  buffer.writeUInt16LE(16, 34); // bits per sample
  buffer.write("data", 36, "ascii");
  buffer.writeUInt32LE(dataBytes, 40);
  for (let i = 0; i < numSamples; i++) {
    const clamped = Math.max(-1, Math.min(1, samples[i]));
    buffer.writeInt16LE(Math.round(clamped * 32767), 44 + i * 2);
  }
  return buffer;
}

// Normalize to a target peak so each asset has predictable headroom.
function normalize(samples, peak = 0.9) {
  let max = 0;
  for (const s of samples) max = Math.max(max, Math.abs(s));
  if (max < 1e-6) return samples;
  const scale = peak / max;
  for (let i = 0; i < samples.length; i++) samples[i] *= scale;
  return samples;
}

// A noise-burst "gunshot": a sharp broadband crack (white noise under a fast
// exponential decay) plus a short low-frequency thump for body.
function gunshot(sampleRate = 44100, duration = 0.45) {
  const n = Math.floor(sampleRate * duration);
  const out = new Float32Array(n);
  const rand = mulberry32(1337);
  for (let i = 0; i < n; i++) {
    const t = i / sampleRate;
    const crack = (rand() * 2 - 1) * Math.exp(-t / 0.07);
    const thump = Math.sin(2 * Math.PI * 55 * t) * Math.exp(-t / 0.13);
    out[i] = crack * 0.9 + thump * 0.6;
  }
  return normalize(out, 0.95);
}

// Additive ambient loop: partials at integer multiples of the loop fundamental
// (so it's exactly periodic = seamless), band-limited to `loBand..hiBand` Hz,
// with a 1/f amplitude tilt and a slow periodic amplitude shimmer.
function ambientLoop({
  sampleRate,
  duration,
  loBand,
  hiBand,
  partials,
  shimmerCycles,
  shimmerDepth,
  seed,
}) {
  const n = Math.floor(sampleRate * duration);
  const fundamental = 1 / duration; // Hz; partial m has frequency m*fundamental
  const out = new Float32Array(n);
  const rand = mulberry32(seed);

  const mLo = Math.max(1, Math.ceil(loBand / fundamental));
  const mHi = Math.floor(hiBand / fundamental);
  const harmonics = [];
  for (let k = 0; k < partials; k++) {
    const m = mLo + Math.floor(rand() * (mHi - mLo));
    const freq = m * fundamental;
    harmonics.push({
      m,
      phase: rand() * 2 * Math.PI,
      amp: 1 / Math.sqrt(freq), // ~1/f tilt: low end dominates
    });
  }

  for (let i = 0; i < n; i++) {
    const t = i / sampleRate;
    let s = 0;
    for (const h of harmonics) {
      s += h.amp * Math.sin(2 * Math.PI * h.m * fundamental * t + h.phase);
    }
    // Shimmer LFO at an integer number of cycles over the loop, so it too is
    // seamless; keeps the bed from sounding static.
    const shimmer =
      1 - shimmerDepth + shimmerDepth * 0.5 * (1 + Math.sin(2 * Math.PI * shimmerCycles * t / duration));
    out[i] = s * shimmer;
  }
  return normalize(out, 0.85);
}

const TARGETS = [
  {
    dest: "examples/mle-lighting/gunshot.wav",
    make: () => ({ samples: gunshot(44100, 0.45), sampleRate: 44100 }),
  },
  {
    dest: "examples/mle-lighting/wind-loop.wav",
    make: () => ({
      samples: ambientLoop({
        sampleRate: 22050,
        duration: 2.0,
        loBand: 60,
        hiBand: 520,
        partials: 60,
        shimmerCycles: 3,
        shimmerDepth: 0.5,
        seed: 7,
      }),
      sampleRate: 22050,
    }),
  },
  {
    dest: "examples/mle-lighting/water-loop.wav",
    make: () => ({
      samples: ambientLoop({
        sampleRate: 22050,
        duration: 2.5,
        loBand: 400,
        hiBand: 4500,
        partials: 140,
        shimmerCycles: 11,
        shimmerDepth: 0.7,
        seed: 23,
      }),
      sampleRate: 22050,
    }),
  },
];

for (const { dest, make } of TARGETS) {
  const { samples, sampleRate } = make();
  const wav = encodeWav(samples, sampleRate);
  await writeFile(path.resolve(dest), wav);
  console.log(`wrote ${dest} (${(wav.length / 1024).toFixed(0)} KB, ${sampleRate} Hz, ${(samples.length / sampleRate).toFixed(2)}s)`);
}
