import './styles/app.css'
import { listen } from '@tauri-apps/api/event'
import { h } from './util'
import { logo, icon } from './ui/icons'
import { ClassesScreen } from './ui/classes'
import { NewsScreen } from './ui/news'

type Section = 'classes' | 'news'

const app = document.getElementById('app')!
const status = h('span', { class: 'status', role: 'status' })
const setStatus = (text: string, cls = '') => {
  status.className = 'status' + (cls ? ' ' + cls : '')
  status.replaceChildren(...(text ? [h('i'), text] : []))
}

const classes = new ClassesScreen(setStatus)
const news = new NewsScreen()
const screens: Record<Section, { el: HTMLElement; load(): Promise<void> }> = { classes, news }

let current: Section = 'classes'
try { const saved = localStorage.getItem('section'); if (saved === 'news' || saved === 'classes') current = saved } catch { /* storage unavailable */ }

const navBtn = (s: Section, label: string, ic: string) =>
  h('button', { 'data-section': s, onclick: () => show(s) }, icon(ic), label)
const nav = h('nav', { 'aria-label': 'Sections' },
  h('div', { class: 'lbl' }, 'Organize'), navBtn('classes', 'Classes', 'cap'),
  h('div', { class: 'lbl' }, 'Monitor'), navBtn('news', 'News', 'feed'))
const shell = h('div', { class: 'shell' }, nav)

function show(s: Section) {
  current = s
  try { localStorage.setItem('section', s) } catch { /* storage unavailable */ }
  for (const b of nav.querySelectorAll<HTMLElement>('button[data-section]')) {
    const on = b.dataset.section === s
    b.classList.toggle('on', on)
    on ? b.setAttribute('aria-current', 'page') : b.removeAttribute('aria-current')
  }
  shell.replaceChildren(nav, screens[s].el)
  void screens[s].load()
}

app.append(h('header', { id: 'top' }, h('div', { class: 'brand' }, logo(), 'CocoaDesk'), h('span', { class: 'sp' }), status), shell)
show(current)
// Classes keep the status line current even while News is open.
if (current !== 'classes') void classes.load()

// The background timer announces changes (classes marked, clock ticks).
void listen('classes:changed', () => void classes.load())
// Refresh when the window comes back, e.g. after the computer wakes up.
document.addEventListener('visibilitychange', () => { if (!document.hidden) void screens[current].load() })
