// Short clicks for picking a card up and setting it down. Each play varies a little in pitch,
// brightness and loudness, and sits left or right with the card, so repeated drags don't sound
// mechanical.
import click1 from '../assets/sounds/click1.ogg?url'
import click2 from '../assets/sounds/click2.ogg?url'

export type Click = 'detach' | 'attach'

const KEY = 'tasks.sound'
export interface SoundPrefs { on: boolean; volume: number }

export function soundPrefs(): SoundPrefs {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) || 'null')
    if (v && typeof v.on === 'boolean' && typeof v.volume === 'number') return { on: v.on, volume: Math.min(1, Math.max(0, v.volume)) }
  } catch { /* storage unavailable */ }
  return { on: true, volume: 0.3 }
}

export function setSoundPrefs(p: SoundPrefs) {
  try { localStorage.setItem(KEY, JSON.stringify(p)) } catch { /* storage unavailable */ }
}

let ctx: AudioContext | null = null
const buffers: Partial<Record<Click, AudioBuffer>> = {}
const lastPitch: Partial<Record<Click, number>> = {}

/** Starts audio. Call from a pointer or key event, which browsers require before playing sound. */
export function primeSound() {
  if (ctx) { if (ctx.state === 'suspended') void ctx.resume(); return }
  ctx = new AudioContext()
  const load = async (kind: Click, url: string) => {
    const data = await (await fetch(url)).arrayBuffer()
    buffers[kind] = await ctx!.decodeAudioData(data)
  }
  void load('detach', click1).catch(() => {})
  void load('attach', click2).catch(() => {})
}

/** Plays a click; `x` is the pointer's position across the window, for stereo placement. */
export function playClick(kind: Click, x = innerWidth / 2) {
  const prefs = soundPrefs()
  const buf = buffers[kind]
  if (!prefs.on || prefs.volume <= 0 || !ctx || !buf) return
  // A random pitch within about ±1.3 semitones, never too close to the previous one.
  let semis = 0
  for (let i = 0; i < 6; i++) {
    semis = (Math.random() * 2 - 1) * 1.3
    const last = lastPitch[kind]
    if (last === undefined || Math.abs(semis - last) > 0.35) break
  }
  lastPitch[kind] = semis
  const base = kind === 'attach' ? -0.5 : 0.6
  const src = ctx.createBufferSource()
  src.buffer = buf
  src.playbackRate.value = Math.pow(2, (base + semis) / 12)
  const tone = ctx.createBiquadFilter()
  tone.type = 'lowpass'
  tone.frequency.value = 6500 + Math.random() * 9000
  tone.Q.value = 0.5
  const gain = ctx.createGain()
  gain.gain.value = prefs.volume * (kind === 'detach' ? 0.8 : 1) * (0.82 + Math.random() * 0.3)
  const pan = ctx.createStereoPanner()
  pan.pan.value = Math.max(-0.4, Math.min(0.4, ((x / Math.max(1, innerWidth)) * 2 - 1) * 0.4))
  src.connect(tone).connect(gain).connect(pan).connect(ctx.destination)
  src.start(ctx.currentTime + Math.random() * 0.006)
}
