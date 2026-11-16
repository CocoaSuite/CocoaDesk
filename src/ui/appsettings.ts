import { invoke } from '@tauri-apps/api/core'
import { h } from '../util'
import { modal, toast } from './dialogs'
import { errText } from '../api'

type AppIconTheme = 'amethyst' | 'midnight'

// Previews of the two app icons, the same artwork the app ships.
const SVG: Record<AppIconTheme, string> = {
  amethyst: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" role="img" aria-label="desk app icon"><rect width="100" height="100" rx="17.5" fill="#251B30"/><g transform="translate(23 21) scale(1.036)"><path d="M44 6C39.5 1.9 34.5 0 28 0C12.5 0 0 12.5 0 28C0 43.5 12.5 56 28 56C34.5 56 39.5 54.1 44 50L36 40C33.8 42 31.3 43 28 43C19.7 43 13 36.3 13 28C13 19.7 19.7 13 28 13C31.3 13 33.8 14 36 16L44 6Z" fill="#BFA3D3"/><rect x="27" y="22" width="25" height="12" rx="2" fill="#BFA3D3"/></g></svg>',
  midnight: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" role="img" aria-label="desk app icon"><rect width="100" height="100" rx="17.5" fill="#141A24"/><g transform="translate(23 21) scale(1.036)"><path d="M44 6C39.5 1.9 34.5 0 28 0C12.5 0 0 12.5 0 28C0 43.5 12.5 56 28 56C34.5 56 39.5 54.1 44 50L36 40C33.8 42 31.3 43 28 43C19.7 43 13 36.3 13 28C13 19.7 19.7 13 28 13C31.3 13 33.8 14 36 16L44 6Z" fill="#8AA7D9"/><rect x="27" y="22" width="25" height="12" rx="2" fill="#8AA7D9"/></g></svg>'
}
const NAMES: [AppIconTheme, string][] = [['amethyst', 'Amethyst'], ['midnight', 'Midnight']]

/** App settings: the app icon and a desktop shortcut. */
export function appSettingsDialog() {
  const pick = h('div', { class: 'iconpick', role: 'radiogroup', 'aria-label': 'App icon' })
  const draw = (cur: AppIconTheme) => {
    pick.replaceChildren()
    for (const [t, label] of NAMES) {
      const art = h('span', { class: 'art' }); art.innerHTML = SVG[t]
      pick.append(h('button', { class: t === cur ? 'on' : '', role: 'radio', 'aria-checked': String(t === cur), onclick: async () => {
        try { draw(await invoke<AppIconTheme>('set_app_icon', { theme: t })) } catch (e) { toast(errText(e)) }
      } }, art, label))
    }
  }
  draw('amethyst')
  invoke<AppIconTheme>('app_icon').then(draw, () => {})
  const m = modal('Settings', [
    h('span', { class: 'lab' }, 'App icon'),
    pick,
    h('div', { class: 'hint' }, 'Used for the window, the notification area and the CocoaDesk shortcuts on this computer.'),
    h('span', { class: 'lab' }, 'Shortcut'),
    h('div', { class: 'fr' }, h('button', { onclick: async (e: MouseEvent) => {
      const b = e.currentTarget as HTMLButtonElement
      b.disabled = true
      try { await invoke('create_shortcut'); toast('Shortcut created on the desktop') } catch (err) { toast(errText(err)) } finally { b.disabled = false }
    } }, 'Create desktop shortcut'), h('span', { class: 'hint' }, 'Uses the icon chosen above.')),
    h('div', { class: 'row' }, h('button', { class: 'pri', onclick: () => m.close() }, 'Done'))
  ])
}
