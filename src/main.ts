import './styles/app.css'
import { listen } from '@tauri-apps/api/event'
import { h } from './util'
import { logo, icon } from './ui/icons'
import { ClassesScreen } from './ui/classes'

const app = document.getElementById('app')!
const status = h('span', { class: 'status', role: 'status' })
const setStatus = (text: string, cls = '') => {
  status.className = 'status' + (cls ? ' ' + cls : '')
  status.replaceChildren(...(text ? [h('i'), text] : []))
}

const classes = new ClassesScreen(setStatus)

app.append(
  h('header', { id: 'top' }, h('div', { class: 'brand' }, logo(), 'CocoaDesk'), h('span', { class: 'sp' }), status),
  h('div', { class: 'shell' },
    h('nav', { 'aria-label': 'Sections' },
      h('div', { class: 'lbl' }, 'Organize'),
      h('button', { class: 'on', 'aria-current': 'page' }, icon('cap'), 'Classes')),
    classes.el))

void classes.load()

// The background timer in the app core announces changes (classes marked, clock ticks).
void listen('classes:changed', () => void classes.load())
// Refresh when the window comes back, e.g. after the computer wakes up.
document.addEventListener('visibilitychange', () => { if (!document.hidden) void classes.load() })
