// The composer's microphone (D15): records here, turns the recording into
// 16 kHz mono 16-bit PCM in the page and hands it to the app, which
// transcribes it on this machine. The audio never leaves it.

/** Longest recording, as the headset's (the device link's MAX_SECONDS). */
export const MAX_SECONDS = 60;
const RATE = 16_000;

/** A recording in progress: `stop()` resolves with its PCM. */
export interface Recording {
  stop(): Promise<Int16Array>;
  /** Drops the recording. */
  cancel(): void;
}

/** Starts recording from the default microphone; it stops itself at MAX_SECONDS. */
export async function record(): Promise<Recording> {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  // The microphone is freed whenever recording stops, however it stops.
  const release = () => stream.getTracks().forEach((t) => t.stop());
  const parts: Blob[] = [];
  let recorder: MediaRecorder;
  let stopped: Promise<void>;
  try {
    recorder = new MediaRecorder(stream);
    recorder.ondataavailable = (e) => e.data.size && parts.push(e.data);
    stopped = new Promise((resolve) => (recorder.onstop = () => (release(), resolve())));
    recorder.start();
  } catch (e) {
    release();
    throw e;
  }
  const end = () => recorder.state === "recording" && recorder.stop();
  const limit = setTimeout(end, MAX_SECONDS * 1000);
  return {
    async stop() {
      clearTimeout(limit);
      end();
      await stopped;
      return pcm(await new Blob(parts, { type: recorder.mimeType }).arrayBuffer());
    },
    cancel() {
      clearTimeout(limit);
      end();
      release();
    },
  };
}

/** `encoded` audio decoded at 16 kHz (the context resamples), mixed to mono 16-bit, at most MAX_SECONDS. */
async function pcm(encoded: ArrayBuffer): Promise<Int16Array> {
  const decoded = await new OfflineAudioContext(1, 1, RATE).decodeAudioData(encoded);
  const frames = Math.min(decoded.length, MAX_SECONDS * RATE);
  const channels = Array.from({ length: decoded.numberOfChannels }, (_, c) => decoded.getChannelData(c));
  return Int16Array.from({ length: frames }, (_, i) => {
    const s = channels.reduce((sum, ch) => sum + ch[i], 0) / channels.length;
    return Math.round(Math.max(-1, Math.min(1, s)) * 32767);
  });
}
