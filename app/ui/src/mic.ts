// The composer's microphone (D15): records here, turns the recording into
// 16 kHz mono 16-bit PCM in the page and hands it to the app, which
// transcribes it on this machine. The audio never leaves it.

/** Longest recording, as the headset's (the device link's MAX_SECONDS). */
export const MAX_SECONDS = 60;
const RATE = 16_000;

/** A recording in progress: `stop()` resolves with its PCM. */
export interface Recording {
  stop(): Promise<Int16Array>;
  /** Drops the recording and frees the microphone. */
  cancel(): void;
}

/** Starts recording from the default microphone. */
export async function record(): Promise<Recording> {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  const recorder = new MediaRecorder(stream);
  const parts: Blob[] = [];
  recorder.ondataavailable = (e) => e.data.size && parts.push(e.data);
  const stopped = new Promise<void>((resolve) => (recorder.onstop = () => resolve()));
  const release = () => stream.getTracks().forEach((t) => t.stop());
  recorder.start();
  const limit = setTimeout(() => recorder.state === "recording" && recorder.stop(), MAX_SECONDS * 1000);
  return {
    async stop() {
      clearTimeout(limit);
      if (recorder.state === "recording") recorder.stop();
      await stopped;
      release();
      return pcm(await new Blob(parts, { type: recorder.mimeType }).arrayBuffer());
    },
    cancel() {
      clearTimeout(limit);
      if (recorder.state === "recording") recorder.stop();
      release();
    },
  };
}

/** `encoded` audio decoded, mixed to mono and resampled to 16 kHz 16-bit. */
async function pcm(encoded: ArrayBuffer): Promise<Int16Array> {
  const context = new AudioContext();
  const decoded = await context.decodeAudioData(encoded).finally(() => context.close());
  const frames = Math.max(1, Math.ceil(decoded.duration * RATE));
  const offline = new OfflineAudioContext(1, frames, RATE);
  const source = offline.createBufferSource();
  source.buffer = decoded;
  source.connect(offline.destination);
  source.start();
  const samples = (await offline.startRendering()).getChannelData(0);
  return Int16Array.from(samples, (s) => Math.round(Math.max(-1, Math.min(1, s)) * 32767));
}
