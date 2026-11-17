import { noteApi, newsApi, errText, type Note } from '../api'
import { h, fmtMinutes } from '../util'
import { icon } from './icons'
import { toast, menuAt } from './dialogs'
import { renderMarkdown, toggleTask, noteTitle, noteSnippet } from './markdown'

/** Most notes side by side in split view. */
const MAX_PANES = 10
type Layout = 'brick' | 'grid'
/** Notes per row in the brick layout, so neighbouring rows are offset like bricks. */
const BRICK: number[][] = [[], [1], [2], [2, 1], [2, 2], [3, 2], [3, 3], [4, 3], [3, 2, 3], [3, 3, 3], [3, 4, 3]]
const LIST_ITEM = /^(\s*)([-*+]|(\d+)([.)]))\s+(\[[ xX]\]\s+)?(.*)$/
const SAVE_DELAY = 500

const dayFmt = new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short' })
function ago(naive: string): string {
  const t = new Date(naive).getTime()
  const mins = Math.max(0, Math.round((Date.now() - t) / 60000))
  if (mins < 1) return 'just now'
  if (mins < 60 * 24) return `${fmtMinutes(mins)} ago`
  const days = Math.round(mins / 1440)
  return days === 1 ? 'yesterday' : days < 7 ? `${days} days ago` : dayFmt.format(new Date(t))
}

/** Inserts text at the selection through the editor, so Ctrl+Z still undoes it. */
function insert(ta: HTMLTextAreaElement, text: string) {
  ta.focus()
  if (!document.execCommand('insertText', false, text)) ta.setRangeText(text, ta.selectionStart, ta.selectionEnd, 'end')
}

/** Selects whole lines around the selection and returns their start and end. */
function selectLines(ta: HTMLTextAreaElement): [number, number] {
  const v = ta.value
  const a = v.lastIndexOf('\n', ta.selectionStart - 1) + 1
  let b = v.indexOf('\n', ta.selectionEnd - (ta.selectionEnd > ta.selectionStart && v[ta.selectionEnd - 1] === '\n' ? 1 : 0))
  if (b < 0) b = v.length
  return [a, b]
}

class Pane {
  el = h('section', { class: 'npane' })
  private title = h('span', { class: 'nt' })
  private view = h('div', { class: 'nview md' })
  ta = h('textarea', { class: 'ned', spellcheck: true, placeholder: 'Write in Markdown — # heading, - list, - [ ] task, **bold**' }) as HTMLTextAreaElement
  private closeBtn: HTMLElement
  editing = false

  constructor(private s: NotesScreen, public id: number) {
    const head = h('div', { class: 'nph', draggable: 'true', title: 'Drag to rearrange · double-click to focus this note' },
      this.title, h('span', { class: 'sp' }),
      h('button', { class: 'ic', title: 'New note beside', 'aria-label': 'New note beside', onclick: () => void s.newNote('', { after: this }) }, icon('split')),
      h('button', { class: 'ic', title: 'More', 'aria-label': 'More', onclick: (e: MouseEvent) => s.noteMenu(this.id, e.currentTarget as HTMLElement) }, icon('more')),
      this.closeBtn = h('button', { class: 'ic', title: 'Close (Ctrl+W)', 'aria-label': 'Close', onclick: () => s.closePane(this) }, icon('x')))
    head.addEventListener('dblclick', e => { if (!(e.target as HTMLElement).closest('button')) s.toggleSolo(this) })
    head.addEventListener('dragstart', e => { e.dataTransfer!.setData('application/x-note-pane', String(s.paneIndex(this))); e.dataTransfer!.effectAllowed = 'move' })
    this.el.append(head, h('div', { class: 'nbody' }, this.view, this.ta))
    this.el.addEventListener('mousedown', () => s.activate(this))
    this.el.addEventListener('dragover', e => {
      const t = e.dataTransfer!.types
      if (t.includes('application/x-note-pane') || t.includes('application/x-note')) { e.preventDefault(); this.el.classList.add('drop') }
    })
    this.el.addEventListener('dragleave', () => this.el.classList.remove('drop'))
    this.el.addEventListener('drop', e => {
      this.el.classList.remove('drop')
      const pane = e.dataTransfer!.getData('application/x-note-pane')
      const note = e.dataTransfer!.getData('application/x-note')
      if (pane) { e.preventDefault(); e.stopPropagation(); s.swapPanes(+pane, s.paneIndex(this)) }
      else if (note) { e.preventDefault(); e.stopPropagation(); s.open(+note, { after: this }) }
    })
    this.view.addEventListener('click', e => this.viewClick(e))
    this.ta.addEventListener('input', () => s.edit(this.id, this.ta.value))
    this.ta.addEventListener('blur', () => this.stopEdit())
    this.ta.addEventListener('keydown', e => this.key(e))
    this.ta.hidden = true
    this.refresh()
  }

  get body() { return this.s.body(this.id) }

  /** Redraws the title and, unless typing, the formatted text. */
  refresh() {
    const t = noteTitle(this.body)
    this.title.textContent = t || 'Untitled'
    this.title.classList.toggle('mute', !t)
    this.closeBtn.hidden = this.s.paneCount() < 2
    if (this.editing) return
    this.view.innerHTML = this.body.trim() ? renderMarkdown(this.body) : '<div class="nempty">Empty note — click to write</div>'
  }

  /** Switches to the Markdown text, with the caret at the start of a source line or at the end. */
  startEdit(line: number | null = null) {
    if (!this.editing) {
      this.editing = true
      this.ta.value = this.body
      this.view.hidden = true
      this.ta.hidden = false
    }
    this.ta.focus()
    const v = this.ta.value
    let pos = v.length
    if (line != null) {
      let start = 0
      for (let i = 0; i < line; i++) { const nx = v.indexOf('\n', start); if (nx < 0) { start = v.length; break } start = nx + 1 }
      const end = v.indexOf('\n', start)
      pos = end < 0 ? v.length : end
    }
    this.ta.setSelectionRange(pos, pos)
    if (line == null) this.ta.scrollTop = this.ta.scrollHeight
  }

  stopEdit() {
    if (!this.editing) return
    this.editing = false
    const top = this.ta.scrollTop / Math.max(1, this.ta.scrollHeight - this.ta.clientHeight)
    this.ta.hidden = true
    this.view.hidden = false
    this.refresh()
    this.view.scrollTop = top * (this.view.scrollHeight - this.view.clientHeight)
    void this.s.flush(this.id)
  }

  private viewClick(e: MouseEvent) {
    const t = e.target as HTMLElement
    const a = t.closest('a')
    if (a) { e.preventDefault(); void newsApi.openLink(a.getAttribute('href')!).catch(err => toast(errText(err))); return }
    if (t instanceof HTMLInputElement && t.dataset.task) {
      this.s.edit(this.id, toggleTask(this.body, +t.dataset.task))
      this.refresh()
      return
    }
    if (window.getSelection()?.toString()) return
    const block = t.closest<HTMLElement>('[data-line]')
    this.startEdit(block ? +block.dataset.line! : null)
  }

  private key(e: KeyboardEvent) {
    const ta = this.ta
    const mod = e.ctrlKey || e.metaKey
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); ta.blur(); return }
    if (e.key === 'Tab') {
      e.preventDefault()
      const [a, b] = selectLines(ta)
      const block = ta.value.slice(a, b)
      const next = e.shiftKey ? block.replace(/^( {1,2}|\t)/gm, '') : block.replace(/^/gm, '  ')
      ta.setSelectionRange(a, b)
      insert(ta, next)
      ta.setSelectionRange(a, a + next.length)
      return
    }
    if (mod && !e.shiftKey && (e.key === 'b' || e.key === 'i')) {
      e.preventDefault()
      const m = e.key === 'b' ? '**' : '*'
      const [a, b] = [ta.selectionStart, ta.selectionEnd]
      const sel = ta.value.slice(a, b)
      insert(ta, m + sel + m)
      if (!sel) ta.setSelectionRange(a + m.length, a + m.length)
      return
    }
    if (mod && e.key === 'Enter') {
      // Ticks the task on this line, or makes the line a task.
      e.preventDefault()
      const [a, b] = selectLines(ta)
      const line = ta.value.slice(a, b)
      const caret = ta.selectionStart
      let next: string
      if (/\[[ xX]\]/.test(line) && LIST_ITEM.test(line)) next = line.replace(/\[([ xX])\]/, (_, c) => (c === ' ' ? '[x]' : '[ ]'))
      else if (LIST_ITEM.test(line)) next = line.replace(/^(\s*([-*+]|\d+[.)])\s+)/, '$1[ ] ')
      else next = line.replace(/^(\s*)/, '$1- [ ] ')
      ta.setSelectionRange(a, b)
      insert(ta, next)
      const pos = Math.min(a + next.length, caret + next.length - line.length)
      ta.setSelectionRange(pos, pos)
      return
    }
    if (e.key === 'Enter' && !e.shiftKey && !mod && ta.selectionStart === ta.selectionEnd) {
      // Carries lists on: a new bullet, the next number or another task box.
      const v = ta.value, p = ta.selectionStart
      const a = v.lastIndexOf('\n', p - 1) + 1
      const m = v.slice(a, p).match(LIST_ITEM)
      if (!m) return
      e.preventDefault()
      const [, ind, bullet, num, sep, task, text] = m
      if (!text.trim() && !v.slice(p, v.indexOf('\n', p) < 0 ? v.length : v.indexOf('\n', p)).trim()) {
        ta.setSelectionRange(a, p)
        insert(ta, '')
        return
      }
      const mark = num ? `${+num + 1}${sep}` : bullet
      insert(ta, `\n${ind}${mark} ${task ? '[ ] ' : ''}`)
    }
  }
}

interface UiState { panes: number[]; active: number; layout: Layout; listOpen: boolean }

/** Quick Markdown notes: a find-or-create list and up to ten notes side by side. */
export class NotesScreen {
  el = h('main', { class: 'notesmain', 'aria-label': 'Notes' })
  private notes = new Map<number, Note>()
  private order: number[] = []
  private panes: Pane[] = []
  private active: Pane | null = null
  private solo: Pane | null = null
  private layout: Layout = 'brick'
  private listOpen = true
  private loaded = false
  private sel = 0
  private timers = new Map<number, ReturnType<typeof setTimeout>>()
  private saving = new Map<number, Promise<void>>()

  private find = h('input', { type: 'search', placeholder: 'Find or create…', 'aria-label': 'Find or create a note', spellcheck: false }) as HTMLInputElement
  private list = h('div', { class: 'nlist', role: 'listbox', 'aria-label': 'Notes' })
  private side = h('aside', { class: 'nside' })
  private grid = h('div', { class: 'ngrid' })
  private count = h('span', { class: 'mute ncount' })
  private layoutBtns = new Map<Layout, HTMLElement>()
  private plusBar: HTMLElement

  constructor() {
    try {
      const st = JSON.parse(localStorage.getItem('notes.ui') || '{}') as Partial<UiState>
      if (st.layout === 'grid') this.layout = 'grid'
      if (st.listOpen === false) this.listOpen = false
    } catch { /* storage unavailable */ }

    const plus = () => h('button', { class: 'ic', title: 'New note (Ctrl+N) — Ctrl+click opens it beside', 'aria-label': 'New note', onclick: (e: MouseEvent) => void this.newNote('', { beside: e.ctrlKey || e.metaKey }) }, icon('plus'))
    this.side.append(h('div', { class: 'nfind' }, this.find, plus()), this.list)
    this.find.addEventListener('input', () => { this.sel = 0; this.renderList() })
    this.find.addEventListener('keydown', e => this.findKey(e))

    const seg = h('div', { class: 'seg', role: 'group', 'aria-label': 'Split layout' }, ...(['brick', 'grid'] as Layout[]).map(l => {
      const b = h('button', { title: l === 'brick' ? 'Brick — rows fill the width, offset like bricks' : 'Grid — equal cells', onclick: () => this.setLayout(l) }, l === 'brick' ? 'Brick' : 'Grid')
      this.layoutBtns.set(l, b)
      return b
    }))
    this.plusBar = plus()
    const bar = h('div', { class: 'ntools' },
      h('button', { class: 'ic', title: 'Show or hide the note list (Ctrl+\\)', 'aria-label': 'Toggle note list', onclick: () => this.toggleList() }, icon('sidebar')),
      this.plusBar, h('span', { class: 'sp' }), this.count, seg)
    this.grid.addEventListener('dragover', e => { if (e.dataTransfer!.types.includes('application/x-note')) e.preventDefault() })
    this.grid.addEventListener('drop', e => {
      const id = e.dataTransfer!.getData('application/x-note')
      if (id) { e.preventDefault(); this.open(+id, { beside: true }) }
    })
    this.el.append(this.side, h('div', { class: 'nright' }, bar, this.grid))
    this.applyChrome()

    document.addEventListener('keydown', e => this.globalKey(e))
    window.addEventListener('beforeunload', () => void this.flushAll())
    document.addEventListener('visibilitychange', () => { if (document.hidden) void this.flushAll() })
  }

  // ---------- data ----------

  body(id: number) { return this.notes.get(id)?.body ?? '' }
  paneCount() { return this.panes.length }
  paneIndex(p: Pane) { return this.panes.indexOf(p) }

  async load(): Promise<void> {
    try {
      const list = await noteApi.list()
      const fresh = new Map(list.map(n => [n.id, n]))
      // Keep text still being typed or saved.
      for (const [id, n] of this.notes) if (fresh.has(id) && (this.timers.has(id) || this.saving.has(id))) fresh.get(id)!.body = n.body
      this.notes = fresh
      this.order = list.map(n => n.id)
      if (!this.loaded) {
        this.loaded = true
        let st: Partial<UiState> = {}
        try { st = JSON.parse(localStorage.getItem('notes.ui') || '{}') } catch { /* storage unavailable */ }
        const ids = (st.panes || []).filter(id => this.notes.has(id)).slice(0, MAX_PANES)
        if (!ids.length && this.order.length) ids.push(this.order[0])
        this.panes = ids.map(id => new Pane(this, id))
        this.active = this.panes[Math.min(st.active ?? 0, this.panes.length - 1)] ?? null
        // Notes left empty earlier and not on screen are tidied away.
        for (const n of list) if (!n.body.trim() && !ids.includes(n.id)) void this.dropNote(n.id)
        this.layoutPanes()
      } else {
        for (const p of [...this.panes]) if (!this.notes.has(p.id)) this.removePane(p)
        this.panes.forEach(p => p.refresh())
        if (!this.panes.length) this.layoutPanes()
      }
      this.renderList()
    } catch (e) {
      this.grid.replaceChildren(h('div', { class: 'fatal' }, 'Could not load notes: ' + errText(e)))
    }
  }

  leave() { this.panes.forEach(p => p.stopEdit()); void this.flushAll() }

  /** Applies typed text: shown at once, saved shortly after. */
  edit(id: number, body: string) {
    const n = this.notes.get(id)
    if (!n || n.body === body) return
    n.body = body
    clearTimeout(this.timers.get(id))
    this.timers.set(id, setTimeout(() => void this.flush(id), SAVE_DELAY))
    for (const p of this.panes) if (p.id === id) p.refresh()
  }

  async flush(id: number): Promise<void> {
    const t = this.timers.get(id)
    if (t === undefined) return this.saving.get(id)
    clearTimeout(t)
    this.timers.delete(id)
    const prev = this.saving.get(id) ?? Promise.resolve()
    const job = prev.then(async () => {
      const n = this.notes.get(id)
      if (!n) return
      try {
        const saved = await noteApi.save(id, n.body)
        const cur = this.notes.get(id)
        if (cur) { cur.updatedAt = saved.updatedAt; this.bump(id) }
      } catch (e) { toast('Could not save the note: ' + errText(e)) }
    })
    this.saving.set(id, job)
    await job
    if (this.saving.get(id) === job) this.saving.delete(id)
  }

  private flushAll() { return Promise.all([...this.timers.keys()].map(id => this.flush(id))) }

  /** Moves a note to the top of the list after it changed. */
  private bump(id: number) {
    const n = this.notes.get(id)!
    this.order = this.order.filter(x => x !== id)
    const firstUnpinned = n.pinned ? 0 : this.order.findIndex(x => !this.notes.get(x)?.pinned)
    this.order.splice(firstUnpinned < 0 ? this.order.length : firstUnpinned, 0, id)
    this.renderList()
  }

  private async dropNote(id: number) {
    clearTimeout(this.timers.get(id))
    this.timers.delete(id)
    this.notes.delete(id)
    this.order = this.order.filter(x => x !== id)
    try { await noteApi.remove(id) } catch { /* already gone */ }
  }

  /** A note left empty when it leaves the screen is deleted rather than kept as "Untitled". */
  private discardIfEmpty(id: number) {
    if (this.notes.has(id) && !this.body(id).trim() && !this.panes.some(p => p.id === id)) { void this.dropNote(id); this.renderList() }
  }

  // ---------- opening and panes ----------

  /** Creates a note and starts writing in it. */
  async newNote(body = '', where: { beside?: boolean; after?: Pane } = {}) {
    if ((where.beside || where.after) && this.panes.length >= MAX_PANES) { toast(`Split view holds up to ${MAX_PANES} notes. Close one first.`); return }
    try {
      const n = await noteApi.create(body)
      this.notes.set(n.id, n)
      this.order.splice(this.order.findIndex(x => !this.notes.get(x)?.pinned) < 0 ? this.order.length : this.order.findIndex(x => !this.notes.get(x)?.pinned), 0, n.id)
      this.open(n.id, where)
      this.paneOf(n.id)?.startEdit()
    } catch (e) { toast(errText(e)) }
  }

  private paneOf(id: number) { return this.panes.find(p => p.id === id) }

  /** Shows a note: in the active pane, or beside it (after a given pane) in split view. */
  open(id: number, where: { beside?: boolean; after?: Pane } = {}): Pane | undefined {
    const there = this.paneOf(id)
    if (there) {
      if (where.after && there !== where.after) { this.panes.splice(this.panes.indexOf(there), 1); this.panes.splice(this.panes.indexOf(where.after) + 1, 0, there); this.layoutPanes() }
      this.activate(there)
      return there
    }
    if (where.beside || where.after || !this.active) {
      if (this.panes.length >= MAX_PANES) { toast(`Split view holds up to ${MAX_PANES} notes. Close one first.`); return }
      const p = new Pane(this, id)
      const at = where.after ? this.panes.indexOf(where.after) + 1 : this.active ? this.panes.indexOf(this.active) + 1 : this.panes.length
      this.panes.splice(at, 0, p)
      this.solo = null
      this.active = p
      this.layoutPanes()
      return p
    }
    // Replace what the active pane shows.
    const old = this.active
    old.stopEdit()
    const p = new Pane(this, id)
    this.panes[this.panes.indexOf(old)] = p
    if (this.solo === old) this.solo = p
    this.active = p
    this.discardIfEmpty(old.id)
    this.layoutPanes()
    return p
  }

  activate(p: Pane) {
    if (this.active === p) return
    this.active = p
    for (const q of this.panes) q.el.classList.toggle('on', q === p)
    this.save()
    this.renderList()
  }

  closePane(p: Pane) {
    p.stopEdit()
    this.removePane(p)
    this.discardIfEmpty(p.id)
    this.renderList()
  }

  private removePane(p: Pane) {
    const i = this.panes.indexOf(p)
    if (i < 0) return
    this.panes.splice(i, 1)
    if (this.solo === p) this.solo = null
    if (this.active === p) this.active = this.panes[Math.min(i, this.panes.length - 1)] ?? null
    this.layoutPanes()
  }

  swapPanes(a: number, b: number) {
    if (a === b || !this.panes[a] || !this.panes[b]) return
    ;[this.panes[a], this.panes[b]] = [this.panes[b], this.panes[a]]
    this.layoutPanes()
  }

  toggleSolo(p: Pane) {
    this.solo = this.solo === p ? null : p
    this.activate(p)
    this.layoutPanes()
  }

  private setLayout(l: Layout) { this.layout = l; this.solo = null; this.layoutPanes() }

  private toggleList(open = !this.listOpen) {
    this.listOpen = open
    this.applyChrome()
    this.save()
    if (open) this.find.focus()
  }

  private applyChrome() {
    this.el.classList.toggle('collapsed', !this.listOpen)
    this.plusBar.hidden = this.listOpen
    for (const [l, b] of this.layoutBtns) b.classList.toggle('on', l === this.layout)
  }

  private save() {
    try {
      const st: UiState = { panes: this.panes.map(p => p.id), active: Math.max(0, this.active ? this.panes.indexOf(this.active) : 0), layout: this.layout, listOpen: this.listOpen }
      localStorage.setItem('notes.ui', JSON.stringify(st))
    } catch { /* storage unavailable */ }
  }

  /** Lays the panes out as bricks (rows that fill the width) or as an even grid. */
  private layoutPanes() {
    const focused = this.panes.find(p => p.editing && document.activeElement === p.ta)
    const caret = focused ? [focused.ta.selectionStart, focused.ta.selectionEnd] : null
    this.applyChrome()
    this.save()
    this.count.textContent = this.panes.length > 1 ? `${this.panes.length} / ${MAX_PANES}` : ''
    for (const p of this.panes) { p.el.classList.toggle('on', p === this.active); p.refresh() }
    this.grid.className = 'ngrid'
    this.grid.style.gridTemplateColumns = this.grid.style.gridTemplateRows = ''
    if (!this.panes.length) {
      this.grid.replaceChildren(h('div', { class: 'empty nstart' },
        h('h2', {}, 'Jot something down'),
        h('div', {}, 'Notes are plain Markdown and save as you type. Ctrl+N starts one from anywhere in Notes, and Ctrl+Shift+N opens a quick note from any section.'),
        h('div', { class: 'row' }, h('button', { class: 'pri', onclick: () => void this.newNote() }, icon('plus'), 'New note'))))
      return
    }
    const shown = this.solo ? [this.solo] : this.panes
    if (this.layout === 'grid' && shown.length > 1) {
      const c = Math.ceil(Math.sqrt(shown.length))
      this.grid.classList.add('grid')
      this.grid.style.gridTemplateColumns = `repeat(${c}, minmax(0, 1fr))`
      this.grid.style.gridTemplateRows = `repeat(${Math.ceil(shown.length / c)}, minmax(0, 1fr))`
      this.grid.replaceChildren(...shown.map(p => p.el))
    } else {
      let i = 0
      this.grid.replaceChildren(...BRICK[shown.length].map(n => h('div', { class: 'nrow' }, ...shown.slice(i, (i += n)).map(p => p.el))))
    }
    if (focused && caret && shown.includes(focused)) { focused.startEdit(); focused.ta.setSelectionRange(caret[0], caret[1]) }
  }

  // ---------- list ----------

  private matches(): Note[] {
    const words = this.find.value.toLowerCase().split(/\s+/).filter(Boolean)
    return this.order.map(id => this.notes.get(id)!).filter(n => n && words.every(w => n.body.toLowerCase().includes(w)))
  }

  /** The rows the find box acts on: an optional "create" row, then matching notes. */
  private rows(): ({ create: string } | Note)[] {
    const q = this.find.value.trim()
    const found = this.matches()
    const exact = q && found.some(n => noteTitle(n.body).toLowerCase() === q.toLowerCase())
    return q && !exact ? [{ create: q }, ...found] : found
  }

  private renderList() {
    if (!this.loaded) return
    const rows = this.rows()
    this.sel = Math.max(0, Math.min(this.sel, rows.length - 1))
    const open = new Set(this.panes.map(p => p.id))
    this.list.replaceChildren(...rows.map((r, i) => {
      if ('create' in r) {
        return h('button', { class: 'nitem create' + (i === this.sel ? ' kb' : ''), role: 'option', onclick: (e: MouseEvent) => this.activateRow(r, e.ctrlKey || e.metaKey) },
          h('b', {}, icon('plus'), `New note “${r.create}”`), h('span', {}, 'Enter'))
      }
      const t = noteTitle(r.body)
      const b = h('button', {
        class: 'nitem' + (this.active?.id === r.id ? ' on' : '') + (i === this.sel ? ' kb' : ''), role: 'option', draggable: 'true',
        title: 'Click to open · Ctrl+click or drag to open beside',
        onclick: (e: MouseEvent) => this.activateRow(r, e.ctrlKey || e.metaKey),
        onauxclick: (e: MouseEvent) => { if (e.button === 1) this.activateRow(r, true) },
        oncontextmenu: (e: MouseEvent) => { e.preventDefault(); this.noteMenu(r.id, { x: e.clientX, y: e.clientY }) },
        ondragstart: (e: DragEvent) => { e.dataTransfer!.setData('application/x-note', String(r.id)); e.dataTransfer!.effectAllowed = 'copy' }
      }, h('b', { class: t ? '' : 'mute' }, r.pinned ? icon('pin', 'i pinned') : null, t || 'Untitled', open.has(r.id) ? h('i', { class: 'odot', title: 'Open' }) : null),
      h('span', {}, ago(r.updatedAt), noteSnippet(r.body) ? ' · ' + noteSnippet(r.body) : ''))
      return b
    }))
    if (!rows.length) this.list.append(h('div', { class: 'hint nnone' }, this.order.length ? 'No note mentions that.' : 'Type to start a note, or press +.'))
  }

  private activateRow(r: { create: string } | Note, beside: boolean) {
    if ('create' in r) {
      this.find.value = ''
      this.renderList()
      void this.newNote(`# ${r.create}\n\n`, { beside })
      return
    }
    const p = this.open(r.id, { beside })
    return p
  }

  private findKey(e: KeyboardEvent) {
    const rows = this.rows()
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      this.sel = (this.sel + (e.key === 'ArrowDown' ? 1 : -1) + rows.length) % Math.max(1, rows.length)
      this.renderList()
      this.list.querySelector('.kb')?.scrollIntoView({ block: 'nearest' })
    } else if (e.key === 'Enter') {
      e.preventDefault()
      const r = rows[this.sel]
      if (!r) { if (!this.find.value.trim()) void this.newNote(); return }
      const p = this.activateRow(r, e.ctrlKey || e.metaKey)
      if (p) { this.find.value = ''; this.renderList(); p.startEdit() }
    } else if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      if (this.find.value) { this.find.value = ''; this.renderList() }
      else this.active?.startEdit()
    }
  }

  noteMenu(id: number, at: HTMLElement | { x: number; y: number }) {
    const n = this.notes.get(id)
    if (!n) return
    menuAt(at, [
      ...(this.paneOf(id) ? [] : [{ label: 'Open beside', hint: 'Ctrl+click', run: () => void this.open(id, { beside: true }) }]),
      { label: n.pinned ? 'Unpin from top' : 'Pin to top', run: () => void this.setPinned(id, !n.pinned) },
      { label: 'Copy as Markdown', run: () => void navigator.clipboard.writeText(n.body).then(() => toast('Copied')) },
      'sep',
      { label: 'Delete note', danger: true, run: () => void this.deleteNote(id) }
    ])
  }

  private async setPinned(id: number, pinned: boolean) {
    try {
      await noteApi.pin(id, pinned)
      this.notes.get(id)!.pinned = pinned
      this.bump(id)
    } catch (e) { toast(errText(e)) }
  }

  private async deleteNote(id: number) {
    const n = this.notes.get(id)
    if (!n) return
    const { body, pinned } = n
    const pane = this.paneOf(id)
    const at = pane ? this.panes.indexOf(pane) : -1
    if (pane) { pane.stopEdit(); this.removePane(pane) }
    await this.dropNote(id)
    this.renderList()
    if (!body.trim()) return
    toast('Note deleted', 6000, {
      label: 'Undo', run: async () => {
        try {
          const back = await noteApi.create(body)
          if (pinned) { await noteApi.pin(back.id, true); back.pinned = true }
          this.notes.set(back.id, back)
          this.order.unshift(back.id)
          this.bump(back.id)
          const after = at > 0 ? this.panes[at - 1] : undefined
          if (at >= 0) this.open(back.id, after ? { after } : { beside: this.panes.length > 0 })
        } catch (e) { toast(errText(e)) }
      }
    })
  }

  // ---------- keys ----------

  private globalKey(e: KeyboardEvent) {
    if (!this.el.isConnected || document.querySelector('.modal,#qnote')) return
    const mod = e.ctrlKey || e.metaKey
    const k = e.key.toLowerCase()
    if (mod && !e.shiftKey && k === 'n') { e.preventDefault(); void this.newNote('', { beside: e.altKey }) }
    else if (mod && k === 'k') { e.preventDefault(); if (!this.listOpen) this.toggleList(true); this.find.focus(); this.find.select() }
    else if (mod && e.key === '\\') { e.preventDefault(); this.toggleList() }
    else if (mod && k === 'w' && this.active) { e.preventDefault(); this.closePane(this.active) }
    else if (e.altKey && !mod && /^Digit\d$/.test(e.code)) {
      const p = this.panes[(+e.code.slice(5) || 10) - 1]
      if (p) { e.preventDefault(); this.activate(p); p.startEdit() }
    }
  }

  // ---------- quick note ----------

  /** A small box for jotting a note from any section. Esc or Ctrl+Enter keeps it. */
  quick(goToNotes: () => void) {
    const existing = document.getElementById('qnote')
    if (existing) { existing.querySelector('textarea')?.focus(); return }
    const ta = h('textarea', { placeholder: 'Jot it down… Markdown works here too', spellcheck: true }) as HTMLTextAreaElement
    const keep = async (open: boolean) => {
      box.remove()
      const body = ta.value.replace(/\s+$/, '')
      if (!body.trim()) { if (open) goToNotes(); return }
      try {
        const n = await noteApi.create(body + '\n')
        if (this.loaded) { this.notes.set(n.id, n); this.order.unshift(n.id); this.bump(n.id) }
        if (open) { goToNotes(); await this.load(); this.open(n.id, { beside: this.panes.length > 0 && this.panes.length < MAX_PANES })?.startEdit() }
        else toast('Saved to Notes', 4000, { label: 'Open', run: () => { goToNotes(); void this.load().then(() => this.open(n.id)) } })
      } catch (e) { toast('Could not save the note: ' + errText(e)) }
    }
    ta.addEventListener('keydown', e => {
      if (e.key === 'Escape' || (e.key === 'Enter' && (e.ctrlKey || e.metaKey))) { e.preventDefault(); e.stopPropagation(); void keep(false) }
    })
    const box = h('div', { id: 'qnote', role: 'dialog', 'aria-label': 'Quick note' },
      h('div', { class: 'qh' }, icon('note'), h('b', {}, 'Quick note'), h('span', { class: 'mute' }, 'Esc saves'), h('span', { class: 'sp' }),
        h('button', { class: 'ic', title: 'Save and open in Notes', 'aria-label': 'Open in Notes', onclick: () => void keep(true) }, icon('split')),
        h('button', { class: 'ic', title: 'Save and close', 'aria-label': 'Save and close', onclick: () => void keep(false) }, icon('check'))),
      ta)
    document.body.append(box)
    ta.focus()
  }
}
