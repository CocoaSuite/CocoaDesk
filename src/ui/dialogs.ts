import { h } from '../util'

let toastEl: HTMLElement | null = null
let toastT: ReturnType<typeof setTimeout> | undefined
export function toast(msg: string, ms = 2800) {
  if (!toastEl) { toastEl = h('div', { id: 'toast', role: 'status' }); document.body.append(toastEl) }
  toastEl.textContent = msg
  toastEl.classList.add('show')
  clearTimeout(toastT)
  toastT = setTimeout(() => toastEl!.classList.remove('show'), ms)
}

export interface ModalCtl<T> { el: HTMLElement; box: HTMLElement; close(v?: T): void; done: Promise<T | undefined> }

export function modal<T = unknown>(title: string, body: (Node | string)[], opts: { size?: 'wide' | 'xwide' } = {}): ModalCtl<T> {
  let resolve!: (v: T | undefined) => void
  const done = new Promise<T | undefined>(r => (resolve = r))
  const box = h('div', { class: 'box' + (opts.size ? ' ' + opts.size : ''), role: 'dialog', 'aria-modal': 'true', 'aria-label': title }, h('h2', {}, title), ...body)
  const el = h('div', { class: 'modal' }, box)
  const prev = document.activeElement as HTMLElement | null
  const close = (v?: T) => { el.remove(); document.removeEventListener('keydown', key, true); prev?.focus?.(); resolve(v) }
  const key = (e: KeyboardEvent) => {
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(undefined) }
  }
  el.addEventListener('mousedown', e => { if (e.target === el) close(undefined) })
  document.addEventListener('keydown', key, true)
  document.body.append(el)
  setTimeout(() => (box.querySelector('input:not([type=checkbox]),select,textarea,button.pri') as HTMLElement | null)?.focus(), 20)
  return { el, box, close, done }
}

export function confirmBox(title: string, text: string, ok = 'Delete', danger = true): Promise<boolean> {
  const m = modal<boolean>(title, [
    h('div', { class: 'hint' }, text),
    h('div', { class: 'row' },
      h('button', { onclick: () => m.close(false) }, 'Cancel'),
      h('button', { class: danger ? 'pri danger' : 'pri', onclick: () => m.close(true) }, ok))
  ])
  return m.done.then(v => !!v)
}

export type MenuItem = { label: string; hint?: string; danger?: boolean; run: () => void } | { header: string } | 'sep'

/** Opens a menu under a button, or at a point (for right-click menus). */
export function menuAt(anchor: HTMLElement | { x: number; y: number }, items: MenuItem[]) {
  document.querySelectorAll('.ctx').forEach(e => e.remove())
  const m = h('div', { class: 'ctx', role: 'menu' })
  const done = () => { m.remove(); document.removeEventListener('mousedown', off, true); document.removeEventListener('keydown', esc, true) }
  for (const it of items) {
    if (it === 'sep') m.append(h('hr'))
    else if ('header' in it) m.append(h('div', { class: 'lbl' }, it.header))
    else m.append(h('button', { role: 'menuitem', class: it.danger ? 'danger' : '', onclick: () => { done(); it.run() } }, it.label, it.hint ? h('span', { class: 'kb' }, it.hint) : null))
  }
  document.body.append(m)
  const r = anchor instanceof HTMLElement ? anchor.getBoundingClientRect() : { left: anchor.x, bottom: anchor.y }
  const mr = m.getBoundingClientRect()
  m.style.top = Math.max(8, Math.min(r.bottom + 4, innerHeight - mr.height - 8)) + 'px'
  m.style.left = Math.max(8, Math.min(r.left, innerWidth - mr.width - 8)) + 'px'
  ;(m.querySelector('button') as HTMLElement | null)?.focus()
  const off = (e: Event) => { if (!m.contains(e.target as Node)) done() }
  const esc = (e: KeyboardEvent) => {
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); done(); if (anchor instanceof HTMLElement) anchor.focus() }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      const bs = [...m.querySelectorAll('button')] as HTMLElement[]
      const i = bs.indexOf(document.activeElement as HTMLElement)
      bs[(i + (e.key === 'ArrowDown' ? 1 : -1) + bs.length) % bs.length]?.focus()
    }
  }
  setTimeout(() => { document.addEventListener('mousedown', off, true); document.addEventListener('keydown', esc, true) })
}

/** A labelled on/off switch in CocoaAuthor's style. */
export function toggle(label: string, hint: string, checked: boolean, onchange: (v: boolean) => void): HTMLElement {
  const inp = h('input', { type: 'checkbox', checked, onchange: () => onchange(inp.checked) }) as HTMLInputElement
  return h('label', { class: 'sw' }, h('span', { class: 'tg' }, inp, h('i')), h('span', { class: 'tx' }, label, hint ? h('span', {}, hint) : null))
}

export function field(label: string, control: HTMLElement): HTMLElement {
  return h('label', { class: 'f' }, label, control)
}

export function select<T extends string | number>(options: [T, string][], value: T, attrs: Record<string, any> = {}): HTMLSelectElement {
  const s = h('select', attrs, ...options.map(([v, l]) => h('option', { value: String(v), selected: v === value }, l))) as HTMLSelectElement
  return s
}
