import './styles/app.css'
import { listen } from '@tauri-apps/api/event'
import { h } from './util'
import { logo, icon } from './ui/icons'
import { ClassesScreen } from './ui/classes'
import { NewsScreen } from './ui/news'
import { ResearchScreen } from './ui/research'
import { BoardsScreen } from './ui/boards'
import { TasksScreen } from './ui/tasks'
import { NotesScreen } from './ui/notes'
import { appSettingsDialog } from './ui/appsettings'

type Section = 'classes' | 'tasks' | 'news' | 'research' | 'boards' | 'notes'

const app = document.getElementById('app')!
const status = h('span', { class: 'status', role: 'status' })
const setStatus = (text: string, cls = '') => {
  status.className = 'status' + (cls ? ' ' + cls : '')
  status.replaceChildren(...(text ? [h('i'), text] : []))
}

const classes = new ClassesScreen(setStatus)
const news = new NewsScreen()
const research = new ResearchScreen()
const boards = new BoardsScreen()
const tasks = new TasksScreen()
const notes = new NotesScreen()
const screens: Record<Section, { el: HTMLElement; load(): Promise<void>; leave?(): void }> = { classes, tasks, news, research, boards, notes }

let current: Section = 'classes'
try { const saved = localStorage.getItem('section'); if (saved === 'news' || saved === 'research' || saved === 'classes' || saved === 'boards' || saved === 'tasks' || saved === 'notes') current = saved } catch { /* storage unavailable */ }

const navBtn = (s: Section, label: string, ic: string) =>
  h('button', { 'data-section': s, onclick: () => show(s) }, icon(ic), label)
const nav = h('nav', { 'aria-label': 'Sections' },
  h('div', { class: 'lbl' }, 'Organize'), navBtn('tasks', 'Tasks', 'tasks'), navBtn('classes', 'Classes', 'cap'),
  h('div', { class: 'lbl' }, 'Collect'), navBtn('notes', 'Notes', 'note'), navBtn('boards', 'Moodboards', 'board'),
  h('div', { class: 'lbl' }, 'Monitor'), navBtn('news', 'News', 'feed'), navBtn('research', 'Research', 'paper'))
const shell = h('div', { class: 'shell' }, nav)

function show(s: Section) {
  if (current !== s) screens[current].leave?.()
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

app.append(h('header', { id: 'top' }, h('div', { class: 'brand' }, logo(), 'CocoaDesk'), h('span', { class: 'sp' }), status,
  h('button', { class: 'ic', title: 'Quick note (Ctrl+Shift+N)', 'aria-label': 'Quick note', onclick: () => quickNote() }, icon('note')),
  h('button', { class: 'ic', title: 'Settings', 'aria-label': 'Settings', onclick: appSettingsDialog }, icon('gear'))), shell)
show(current)
// Classes keep the status line current even while News is open.
if (current !== 'classes') void classes.load()

// The background timer announces changes (classes marked, clock ticks).
void listen('classes:changed', () => void classes.load())
// Watched research topics are checked in the background too.
void listen<number>('research:checking', e => research.checking(e.payload))
void listen<number>('research:changed', e => research.changed(e.payload))
// Refresh when the window comes back, e.g. after the computer wakes up.
document.addEventListener('visibilitychange', () => { if (!document.hidden) void screens[current].load() })

// Quick note from anywhere: Ctrl+Shift+N, the top bar or the tray menu.
const quickNote = () => notes.quick(() => show('notes'))
document.addEventListener('keydown', e => {
  if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === 'n' && !document.querySelector('.modal')) { e.preventDefault(); quickNote() }
})
void listen('notes:quick', () => quickNote())
