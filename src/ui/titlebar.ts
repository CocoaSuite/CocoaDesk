import { getCurrentWindow } from '@tauri-apps/api/window'
import { h } from '../util'

/*
 * Window controls in the top bar, matching CocoaShot and CocoaAuthor.
 * The window has no native frame (decorations: false); the top bar is the drag area
 * and the minimise / maximise / close buttons sit flush at its right end.
 */

const NS = 'http://www.w3.org/2000/svg'
const glyph = (d: string) => {
  const s = document.createElementNS(NS, 'svg')
  s.setAttribute('viewBox', '0 0 10 10')
  s.setAttribute('aria-hidden', 'true')
  const p = document.createElementNS(NS, 'path')
  p.setAttribute('d', d)
  s.append(p)
  return s
}
const MIN = 'M0 5.5h10'
const MAX = 'M.5.5h9v9h-9z'
const RESTORE = 'M.5 2.5h7v7h-7zM2.5 2.5V.5h7v7h-2'
const CLOSE = 'M.5.5l9 9M9.5.5l-9 9'

const INTERACTIVE = 'button,input,select,textarea,a,label,[contenteditable],[data-nodrag]'

export function windowControls(bar: HTMLElement): HTMLElement {
  const win = getCurrentWindow()
  const maxBtn = h('button', { class: 'wc', onclick: () => void win.toggleMaximize() })
  const setMax = (on: boolean) => {
    maxBtn.replaceChildren(glyph(on ? RESTORE : MAX))
    maxBtn.title = on ? 'Restore down' : 'Maximize'
    maxBtn.setAttribute('aria-label', maxBtn.title)
  }
  const sync = () => { void win.isMaximized().then(setMax).catch(() => { /* window gone */ }) }
  setMax(false)
  sync()
  void win.onResized(sync)
  void win.onFocusChanged(({ payload }) => document.documentElement.classList.toggle('blur', !payload))

  // Drag the window by any empty part of the bar; double-click maximises, as with a native title bar.
  bar.addEventListener('mousedown', e => {
    if (e.button !== 0 || (e.target as Element).closest(INTERACTIVE)) return
    e.preventDefault()
    if (e.detail === 2) void win.toggleMaximize()
    else void win.startDragging()
  })

  document.documentElement.classList.add('wco')
  return h('div', { class: 'wctl', role: 'group', 'aria-label': 'Window' },
    h('button', { class: 'wc', title: 'Minimize', 'aria-label': 'Minimize', onclick: () => void win.minimize() }, glyph(MIN)),
    maxBtn,
    h('button', { class: 'wc close', title: 'Close', 'aria-label': 'Close', onclick: () => void win.close() }, glyph(CLOSE)))
}
