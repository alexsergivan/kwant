// Web Audio API pure 8-bit chiptune sound synthesizer

let audioCtx: AudioContext | null = null;
let isMuted = localStorage.getItem('kwnt_sfx_muted') === 'true';

function getAudioContext(): AudioContext | null {
  if (typeof window === 'undefined') return null;
  if (!audioCtx) {
    const AudioContextClass = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
    if (AudioContextClass) {
      audioCtx = new AudioContextClass();
    }
  }
  if (audioCtx && audioCtx.state === 'suspended') {
    audioCtx.resume();
  }
  return audioCtx;
}

export function isSfxMuted(): boolean {
  return isMuted;
}

export function toggleSfxMute(): boolean {
  isMuted = !isMuted;
  localStorage.setItem('kwnt_sfx_muted', String(isMuted));
  if (!isMuted) {
    playSfx('blip');
  }
  return isMuted;
}

export type SfxType = 'blip' | 'coin' | 'alert' | 'fanfare' | 'click';

export function playSfx(type: SfxType) {
  if (isMuted) return;

  try {
    const ctx = getAudioContext();
    if (!ctx) return;

    const now = ctx.currentTime;

    switch (type) {
      case 'click': {
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        osc.type = 'triangle';
        osc.frequency.setValueAtTime(400, now);
        osc.frequency.exponentialRampToValueAtTime(120, now + 0.03);
        gain.gain.setValueAtTime(0.12, now);
        gain.gain.exponentialRampToValueAtTime(0.001, now + 0.03);
        osc.connect(gain);
        gain.connect(ctx.destination);
        osc.start(now);
        osc.stop(now + 0.03);
        break;
      }

      case 'blip': {
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        osc.type = 'square';
        osc.frequency.setValueAtTime(880, now);
        gain.gain.setValueAtTime(0.06, now);
        gain.gain.exponentialRampToValueAtTime(0.001, now + 0.04);
        osc.connect(gain);
        gain.connect(ctx.destination);
        osc.start(now);
        osc.stop(now + 0.04);
        break;
      }

      case 'coin': {
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        osc.type = 'square';
        osc.frequency.setValueAtTime(987.77, now); // B5
        osc.frequency.setValueAtTime(1318.51, now + 0.08); // E6
        gain.gain.setValueAtTime(0.15, now);
        gain.gain.setValueAtTime(0.15, now + 0.08);
        gain.gain.exponentialRampToValueAtTime(0.001, now + 0.28);
        osc.connect(gain);
        gain.connect(ctx.destination);
        osc.start(now);
        osc.stop(now + 0.28);
        break;
      }

      case 'alert': {
        // 8-bit battle / approval needed siren
        for (let i = 0; i < 2; i++) {
          const osc = ctx.createOscillator();
          const gain = ctx.createGain();
          osc.type = 'sawtooth';
          const t = now + i * 0.12;
          osc.frequency.setValueAtTime(440, t);
          osc.frequency.setValueAtTime(880, t + 0.06);
          gain.gain.setValueAtTime(0.18, t);
          gain.gain.exponentialRampToValueAtTime(0.001, t + 0.11);
          osc.connect(gain);
          gain.connect(ctx.destination);
          osc.start(t);
          osc.stop(t + 0.11);
        }
        break;
      }

      case 'fanfare': {
        // Victory fanfare arpeggio: G4, C5, E5, G5
        const notes = [392.00, 523.25, 659.25, 783.99];
        notes.forEach((freq, idx) => {
          const osc = ctx.createOscillator();
          const gain = ctx.createGain();
          osc.type = 'square';
          const t = now + idx * 0.09;
          osc.frequency.setValueAtTime(freq, t);
          gain.gain.setValueAtTime(0.16, t);
          gain.gain.exponentialRampToValueAtTime(0.001, t + (idx === 3 ? 0.35 : 0.08));
          osc.connect(gain);
          gain.connect(ctx.destination);
          osc.start(t);
          osc.stop(t + (idx === 3 ? 0.35 : 0.08));
        });
        break;
      }
    }
  } catch (err) {
    console.debug('SFX audio playback error (ignored):', err);
  }
}
