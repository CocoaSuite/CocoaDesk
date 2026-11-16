import { boardApi, boardImageUrl, errText, type BoardSummary } from '../api'
import { h } from '../util'
import { icon } from './icons'
import { toast, modal, confirmBox, field, menuAt } from './dialogs'
import { BoardEditor, COLORS, WIDTHS, type Tool } from './board/editor'

const TOOLS: [Tool, string, string, string][] = [
  ['select', 'cursor', 'Select and move', 'V'],
  ['hand', 'hand', 'Pan — or hold Space, or drag with the middle button', 'H'],
  ['section', 'frame', 'Section', 'S'],
  ['text', 'text', 'Text', 'T'],
  ['pen', 'pen', 'Pen', 'P'],
  ['arrow', 'arrow', 'Arrow — Shift keeps it to 45°', 'A'],
  ['rect', 'rect', 'Rectangle — Shift for a square', 'R'],
  ['ellipse', 'ellipse', 'Ellipse — Shift for a circle', 'O'],
  ['eraser', 'eraser', 'Eraser — click or swipe over drawings', 'E']
]
const GROUPS: Tool[][] = [['select', 'hand'], ['section', 'text'], ['pen', 'arrow', 'rect', 'ellipse', 'eraser']]

const SHORTCUTS: [string, string][] = [
  ['Ctrl+V', 'Paste images, copied web pictures, links or text'],
  ['Wheel', 'Zoom at the pointer'],
  ['Space + drag, middle drag', 'Pan'],
  ['Shift+1, Shift+2, Ctrl+0', 'Fit everything, zoom to selection, 100%'],
  ['Double-click a picture', 'Zoom to it'],
  ['Drag a corner', 'Resize (pictures keep their shape; Shift to stretch)'],
  ['Drag the round handle', 'Rotate (Shift: 15° steps)'],
  ['Alt + drag', 'Move a copy'],
  ['Ctrl+D, Ctrl+G', 'Duplicate, put the selection in a section'],
  ['] and [', 'Bring to front, send to back'],
  ['Arrows (Shift)', 'Nudge by 1 (10) pixels'],
  ['Ctrl+Z, Ctrl+Y', 'Undo, redo'],
  ['Double-click text or a section name', 'Edit it'],
  ['V H S T P A R O E', 'Tools, in toolbar order'],
  ['F11, Esc', 'Full screen, leave full screen'],
  ['Esc', 'Back to selecting; clear the selection']
]

/** Moodboards: several boards of reference pictures, notes and sketches. */
export class BoardsScreen {
  el = h('main', { class: 'boardmain', 'aria-label': 'Moodboards' })
  private editor = new BoardEditor()
  private boards: BoardSummary[] = []
  private boardId: number | null = null
  private loaded = false
  private pinned = false
  private full = false
  private fullTimer: ReturnType<typeof setTimeout> | undefined
  private saveTimer: ReturnType<typeof setTimeout> | undefined
  private dirty = false
  private saving: Promise<void> = Promise.resolve()

  private bar = h('div', { class: 'btools', role: 'toolbar', 'aria-label': 'Board tools' })
  private pinbar = h('div', { class: 'pinbar' })
  private fullbar = h('div', { class: 'fullbar' })
  private wrap = h('div', { class: 'bwrap' })
  private nameBtn = h('button', { class: 'bname', 'aria-haspopup': 'true', onclick: () => this.openPicker() })
  private zoomLbl = h('button', { class: 'zl', title: 'Zoom options', onclick: () => this.zoomMenu() })
  private undoBtn = h('button', { class: 'ic', title: 'Undo (Ctrl+Z)', 'aria-label': 'Undo', onclick: () => this.editor.undo() }, icon('undo'))
  private redoBtn = h('button', { class: 'ic', title: 'Redo (Ctrl+Y)', 'aria-label': 'Redo', onclick: () => this.editor.redo() }, icon('redo'))
  private toolBtns = new Map<Tool, HTMLElement>()
  private colorBtns = new Map<string, HTMLElement>()
  private widthBtns = new Map<number, HTMLElement>()

  constructor() {
    this.editor.onChange = what => this.scheduleSave(what === 'scene' ? 400 : 1500)
    this.editor.onState = () => this.refreshBar()
    this.buildBar()
    this.wrap.append(this.editor.el, this.pinbar, this.fullbar)
    document.addEventListener('keydown', e => this.fullKey(e), true)
    window.addEventListener('beforeunload', () => void this.flush())
    document.addEventListener('visibilitychange', () => { if (document.hidden) void this.flush() })
  }

  private buildBar() {
    const tools = GROUPS.map(g => h('div', { class: 'seg' }, ...g.map(t => {
      const [, ic, label, key] = TOOLS.find(x => x[0] === t)!
      const b = h('button', { title: `${label} (${key})`, 'aria-label': label, onclick: () => this.editor.setTool(t) }, icon(ic))
      this.toolBtns.set(t, b)
      return b
    })))
    const colors = h('div', { class: 'swatches', role: 'group', 'aria-label': 'Colour' }, ...COLORS.map(([c, name]) => {
      const b = h('button', { class: 'sw-c', title: name, 'aria-label': name, style: `--c:${c}`, onclick: () => this.editor.setColor(c) })
      this.colorBtns.set(c, b)
      return b
    }))
    const widths = h('div', { class: 'seg widths', role: 'group', 'aria-label': 'Line thickness' }, ...WIDTHS.map(([w, name]) => {
      const b = h('button', { title: `${name} line`, 'aria-label': `${name} line`, onclick: () => this.editor.setWidth(w) }, h('i', { style: { height: Math.max(2, w - 1) + 'px' } }))
      this.widthBtns.set(w, b)
      return b
    }))
    this.bar.append(
      this.nameBtn, h('span', { class: 'vsep' }), ...tools, h('span', { class: 'vsep' }), colors, widths,
      h('span', { class: 'sp' }),
      this.undoBtn, this.redoBtn,
      h('div', { class: 'seg zoom' },
        h('button', { title: 'Zoom out (Ctrl+−)', 'aria-label': 'Zoom out', onclick: () => this.editor.zoomBy(0.8) }, icon('minus')),
        this.zoomLbl,
        h('button', { title: 'Zoom in (Ctrl+=)', 'aria-label': 'Zoom in', onclick: () => this.editor.zoomBy(1.25) }, icon('plus'))),
      h('button', { onclick: () => this.editor.pickFiles(), title: 'Add pictures from your computer', 'aria-label': 'Add images' }, icon('image'), h('span', { class: 'bl' }, 'Add images')),
      h('button', { onclick: () => void this.setPinned(true), title: 'Pin on top: keep a small board window above other apps', 'aria-label': 'Pin on top' }, icon('pin'), h('span', { class: 'bl' }, 'Pin on top')),
      h('button', { class: 'ic', title: 'Full screen: hide everything but the board (F11)', 'aria-label': 'Full screen', onclick: () => this.setFull(true) }, icon('full')),
      h('button', { class: 'ic', title: 'Keyboard shortcuts', 'aria-label': 'Keyboard shortcuts', onclick: () => this.showHelp() }, icon('help')))
    this.pinbar.append(
      h('button', { onclick: () => void this.setPinned(false), title: 'Back to the full window' }, icon('pin'), 'Unpin'),
      h('button', { class: 'ic', title: 'Fit everything (Shift+1)', 'aria-label': 'Fit everything', onclick: () => this.editor.fitAll() }, icon('fit')),
      h('button', { class: 'ic', title: 'Add images', 'aria-label': 'Add images', onclick: () => this.editor.pickFiles() }, icon('image')))
    this.fullbar.append(
      h('button', { onclick: () => { this.setFull(false, true); void this.setPinned(true) }, title: 'Pin on top: keep a small board window above other apps' }, icon('pin'), 'Pin on top'),
      h('button', { class: 'ic exit', title: 'Exit full screen (Esc or F11)', 'aria-label': 'Exit full screen', onclick: () => this.setFull(false) }, icon('unfull')))
  }

  private refreshBar() {
    const ed = this.editor
    for (const [t, b] of this.toolBtns) { b.classList.toggle('on', ed.tool === t); b.setAttribute('aria-pressed', String(ed.tool === t)) }
    for (const [c, b] of this.colorBtns) b.classList.toggle('on', ed.color === c)
    for (const [w, b] of this.widthBtns) b.classList.toggle('on', ed.width === w)
    this.zoomLbl.textContent = Math.round(ed.view.zoom * 100) + '%'
    ;(this.undoBtn as HTMLButtonElement).disabled = !ed.canUndo
    ;(this.redoBtn as HTMLButtonElement).disabled = !ed.canRedo
  }

  // ---------- loading and saving ----------

  async load(): Promise<void> {
    try {
      this.boards = await boardApi.list()
    } catch (e) {
      this.el.replaceChildren(h('div', { class: 'fatal' }, errText(e)))
      return
    }
    if (!this.boards.length) { this.boardId = null; this.loaded = false; this.renderEmpty(); return }
    if (!this.el.contains(this.wrap)) this.el.replaceChildren(this.bar, this.wrap)
    // Only (re)open a board when none is open: the one on screen may have unsaved changes.
    if (this.loaded && this.boards.some(b => b.id === this.boardId)) { this.refreshName(); return }
    let want: number | null = null
    try { want = Number(localStorage.getItem('board')) || null } catch { /* storage unavailable */ }
    await this.open(this.boards.find(b => b.id === want)?.id ?? this.boards[0].id)
  }

  private async open(id: number) {
    await this.flush()
    try {
      const b = await boardApi.load(id)
      this.boardId = b.id
      this.loaded = true
      try { localStorage.setItem('board', String(b.id)) } catch { /* storage unavailable */ }
      if (!this.el.contains(this.wrap)) this.el.replaceChildren(this.bar, this.wrap)
      this.editor.load(b.scene)
      this.refreshName()
    } catch (e) { toast(errText(e)) }
  }

  private refreshName() {
    const b = this.boards.find(x => x.id === this.boardId)
    this.nameBtn.replaceChildren(icon('board'), h('span', {}, b?.name ?? 'Board'), icon('down', 'i chev'))
  }

  private scheduleSave(ms: number) {
    this.dirty = true
    clearTimeout(this.saveTimer)
    this.saveTimer = setTimeout(() => void this.flush(), ms)
  }

  /** Writes pending changes now. */
  flush(): Promise<void> {
    clearTimeout(this.saveTimer)
    if (!this.dirty || this.boardId == null) return this.saving
    this.dirty = false
    const id = this.boardId, scene = this.editor.sceneJson()
    this.saving = this.saving.then(() => boardApi.save(id, scene)).catch(e => { toast(`Couldn't save the board: ${errText(e)}`, 5000) })
    return this.saving
  }

  /** Called when another section is opened. */
  leave() {
    this.setFull(false, true)
    this.editor.finishEdit()
    void this.flush()
  }

  private renderEmpty() {
    this.el.replaceChildren(h('div', { class: 'bempty' }, h('div', { class: 'empty' },
      h('h2', {}, 'Moodboards'),
      h('div', {}, 'Collect reference pictures on an endless board: paste screenshots, drop files or pictures from your browser, then arrange, resize and rotate them. Group them into sections and add notes and arrows.'),
      h('div', { class: 'row' }, h('button', { class: 'pri', onclick: () => void this.newBoard() }, icon('plus'), 'Create a board')))))
  }

  // ---------- boards ----------

  private async newBoard() {
    const name = await this.askName('New board', `Board ${this.boards.length + 1}`, 'Create')
    if (!name) return
    try {
      await this.flush()
      const id = await boardApi.create(name)
      this.boards = await boardApi.list()
      await this.open(id)
    } catch (e) { toast(errText(e)) }
  }

  private askName(title: string, value: string, ok: string): Promise<string | undefined> {
    const input = h('input', { type: 'text', value, maxlength: 120 }) as HTMLInputElement
    const err = h('div', { class: 'err' })
    const m = modal<string>(title, [
      field('Name', input), err,
      h('div', { class: 'row' }, h('button', { onclick: () => m.close() }, 'Cancel'), h('button', { class: 'pri', onclick: () => submit() }, ok))
    ])
    const submit = () => { const v = input.value.trim(); if (!v) { err.textContent = 'Give the board a name.'; return } m.close(v) }
    input.addEventListener('keydown', e => { if (e.key === 'Enter') submit() })
    setTimeout(() => { input.focus(); input.select() }, 30)
    return m.done
  }

  private async openPicker() {
    await this.flush()
    try { this.boards = await boardApi.list() } catch { /* keep the old list */ }
    document.querySelectorAll('.ctx').forEach(e => e.remove())
    const cur = this.boards.find(b => b.id === this.boardId)
    const panel = h('div', { class: 'ctx bpick', role: 'menu' })
    const close = () => { panel.remove(); document.removeEventListener('mousedown', off, true); document.removeEventListener('keydown', esc, true) }
    const off = (e: Event) => { if (!panel.contains(e.target as Node) && e.target !== this.nameBtn && !this.nameBtn.contains(e.target as Node)) close() }
    const esc = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); this.nameBtn.focus() }
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        const bs = [...panel.querySelectorAll('button')] as HTMLElement[]
        const i = bs.indexOf(document.activeElement as HTMLElement)
        bs[(i + (e.key === 'ArrowDown' ? 1 : -1) + bs.length) % bs.length]?.focus()
      }
    }
    const act = (fn: () => void) => () => { close(); fn() }
    panel.append(h('div', { class: 'lbl' }, 'Boards'))
    for (const b of this.boards) {
      panel.append(h('button', { role: 'menuitem', class: b.id === this.boardId ? 'cur' : '', onclick: act(() => void this.open(b.id)) },
        b.cover ? h('img', { class: 'cov', src: boardImageUrl(b.cover), alt: '' }) : h('span', { class: 'cov' }, icon('board')),
        h('span', { class: 'bn' }, h('b', {}, b.name), h('span', {}, b.items === 1 ? '1 item' : `${b.items} items`))))
    }
    panel.append(h('button', { role: 'menuitem', onclick: act(() => void this.newBoard()) }, h('span', { class: 'cov' }, icon('plus')), 'New board'))
    if (cur) {
      panel.append(h('hr'),
        h('button', { role: 'menuitem', onclick: act(() => void this.rename(cur)) }, 'Rename…'),
        h('button', { role: 'menuitem', onclick: act(() => void this.duplicateBoard(cur)) }, 'Duplicate board'),
        h('button', { role: 'menuitem', class: 'danger', onclick: act(() => void this.deleteBoard(cur)) }, 'Delete board…'))
    }
    document.body.append(panel)
    const r = this.nameBtn.getBoundingClientRect()
    panel.style.top = r.bottom + 4 + 'px'
    panel.style.left = r.left + 'px'
    ;(panel.querySelector('button.cur') as HTMLElement | null ?? panel.querySelector('button') as HTMLElement | null)?.focus()
    setTimeout(() => { document.addEventListener('mousedown', off, true); document.addEventListener('keydown', esc, true) })
  }

  private async rename(b: BoardSummary) {
    const name = await this.askName('Rename board', b.name, 'Rename')
    if (!name || name === b.name) return
    try { await boardApi.rename(b.id, name); this.boards = await boardApi.list(); this.refreshName() } catch (e) { toast(errText(e)) }
  }

  private async duplicateBoard(b: BoardSummary) {
    try {
      await this.flush()
      const id = await boardApi.duplicate(b.id)
      this.boards = await boardApi.list()
      await this.open(id)
      toast(`Made a copy of «${b.name}».`)
    } catch (e) { toast(errText(e)) }
  }

  private async deleteBoard(b: BoardSummary) {
    const n = b.items
    if (!(await confirmBox('Delete board', `«${b.name}» and ${n === 1 ? 'its 1 item' : `its ${n} items`} will be deleted. This can't be undone.`))) return
    try {
      clearTimeout(this.saveTimer); this.dirty = false
      await this.saving
      await boardApi.remove(b.id)
      this.loaded = false
      try { localStorage.removeItem('board') } catch { /* storage unavailable */ }
      await this.load()
    } catch (e) { toast(errText(e)) }
  }

  // ---------- view ----------

  private zoomMenu() {
    const ed = this.editor
    menuAt(this.zoomLbl, [
      { label: 'Fit everything', hint: 'Shift+1', run: () => ed.fitAll() },
      { label: 'Zoom to selection', hint: 'Shift+2', run: () => ed.fitSelection() },
      { label: 'Actual size (100%)', hint: 'Ctrl+0', run: () => ed.zoomTo(1) },
      { label: '50%', run: () => ed.zoomTo(0.5) },
      { label: '200%', run: () => ed.zoomTo(2) }
    ])
  }

  private async setPinned(on: boolean) {
    if (on === this.pinned) return
    try {
      await boardApi.overlay(on)
    } catch (e) { toast(errText(e)); return }
    this.pinned = on
    document.body.classList.toggle('pinned', on)
    if (!on) this.editor.el.focus({ preventScroll: true })
    // Frame the work once the window has settled at its new size.
    for (const ms of [120, 450]) setTimeout(() => this.editor.fitSelection(), ms)
  }

  /** Full screen: the interface slides away and the board fills the window. */
  private setFull(on: boolean, instant = false) {
    if (on && (this.pinned || !this.el.contains(this.wrap))) return
    if (on === this.full && !(instant && document.body.classList.contains('bfull'))) return
    this.full = on
    clearTimeout(this.fullTimer)
    const body = document.body.classList
    const r0 = this.editor.el.getBoundingClientRect()
    const settle = () => {
      body.remove('bfull')
      const r1 = this.editor.el.getBoundingClientRect()
      this.editor.panBy(r0.left - r1.left, r0.top - r1.top)
    }
    if (on) {
      body.add('bfull')
      const r1 = this.editor.el.getBoundingClientRect()
      this.editor.panBy(r0.left - r1.left, r0.top - r1.top)
      requestAnimationFrame(() => requestAnimationFrame(() => { if (this.full) body.add('bhide') }))
    } else {
      body.remove('bhide')
      if (instant) settle()
      else this.fullTimer = setTimeout(settle, 280)
    }
    this.editor.el.focus({ preventScroll: true })
  }

  private fullKey(e: KeyboardEvent) {
    if (!this.el.isConnected || e.ctrlKey || e.metaKey || e.altKey) return
    if (e.key === 'F11') { e.preventDefault(); e.stopPropagation(); this.setFull(!this.full); return }
    if (e.key !== 'Escape' || !this.full || document.querySelector('.modal,.ctx')) return
    const t = e.target as HTMLElement | null
    if (t?.isContentEditable || t?.tagName === 'INPUT' || t?.tagName === 'TEXTAREA') return
    e.preventDefault(); e.stopPropagation()
    this.setFull(false)
  }

  private showHelp() {
    const m = modal('Moodboard shortcuts', [
      h('div', { class: 'keys' }, ...SHORTCUTS.flatMap(([k, d]) => [h('kbd', {}, k), h('span', {}, d)])),
      h('div', { class: 'hint' }, 'Drag a section by its name. Whatever sits inside a section moves with it.'),
      h('div', { class: 'row' }, h('button', { class: 'pri', onclick: () => m.close() }, 'Done'))
    ], { size: 'wide' })
  }
}
