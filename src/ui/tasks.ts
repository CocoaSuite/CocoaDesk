import { listen } from '@tauri-apps/api/event'
import { taskApi, errText, type Card, type ListView, type TasksView } from '../api'
import { h, isoDate, addDays } from '../util'
import { icon } from './icons'
import { toast, menuAt, confirmBox, promptBox, type MenuItem } from './dialogs'
import { cardDialog } from './taskcard'
import { boardDialog } from './taskboard'
import { fmtUsd, fmtRub, fmtIn, emptySum, addMoney, fmtSumMain, fmtSumOther, type Sum } from './money'
import { primeSound, playClick } from './sound'

const dayFmt = new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short' })
const monthFmt = new Intl.DateTimeFormat('en-GB', { month: 'long' })
const monthYearFmt = new Intl.DateTimeFormat('en-GB', { month: 'long', year: 'numeric' })

const store = {
  get: (k: string) => { try { return localStorage.getItem(k) } catch { return null } },
  set: (k: string, v: string) => { try { localStorage.setItem(k, v) } catch { /* storage unavailable */ } }
}

/** A card being dragged. */
interface Drag {
  card: Card
  el: HTMLElement
  ghost: HTMLElement
  slot: HTMLElement
  fromList: number
  fromIndex: number
  offX: number
  offY: number
  w: number
  h: number
  tilt: number
  vx: number
  lastX: number
  lift: number
  over: { list: number; index: number }
  dropping: boolean
}

const thisMonth = () => isoDate(new Date()).slice(0, 7)

export class TasksScreen {
  el = h('main', { 'aria-label': 'Tasks', class: 'tasks' })
  private view: TasksView | null = null
  private boardId: number | null = Number(store.get('tasks.board')) || null
  private panelOpen = store.get('tasks.earnings') !== '0'
  private loading: Promise<void> | null = null
  private listsEl: HTMLElement | null = null
  private adding: number | null = null
  private addingList = false

  private pending: { card: Card; el: HTMLElement; x: number; y: number } | null = null
  private drag: Drag | null = null
  private ptr = { x: 0, y: 0 }
  private raf = 0
  private eatClick = false
  private reloadAfterDrag = false
  /** A move already shown on screen but not yet in the loaded figures. */
  private settled: Drag | null = null

  constructor() {
    void listen('tasks:changed', () => { if (this.el.isConnected) void this.load() })
    addEventListener('pointermove', e => this.move(e))
    addEventListener('pointerup', () => this.up())
    addEventListener('pointercancel', () => this.up())
    addEventListener('click', e => { if (this.eatClick) { e.preventDefault(); e.stopPropagation(); this.eatClick = false } }, true)
    document.addEventListener('keydown', e => {
      if (e.key === 'Escape' && this.drag && !this.drag.dropping) { e.preventDefault(); this.cancelDrag() }
    }, true)
  }

  async load(): Promise<void> {
    if (this.drag || this.pending) { this.reloadAfterDrag = true; return }
    if (this.loading) return this.loading
    this.loading = (async () => {
      try {
        this.view = await taskApi.view(this.boardId)
        this.boardId = this.view.board?.id ?? null
        if (this.boardId) store.set('tasks.board', String(this.boardId))
        this.settled = null
        if (!this.drag) this.render()
      } catch (e) {
        this.el.replaceChildren(h('div', { class: 'fatal' }, 'Could not load your tasks: ' + errText(e)))
      } finally { this.loading = null }
    })()
    return this.loading
  }

  leave() { this.cancelDrag() }

  // ---------- drawing ----------

  private render() {
    const v = this.view!
    const b = v.board
    if (!b) return
    const scroll = this.listsEl ? { left: this.listsEl.scrollLeft, lists: [...this.listsEl.querySelectorAll<HTMLElement>('.tcards')].map(e => e.scrollTop) } : null
    const focusKey = (document.activeElement as HTMLElement | null)?.dataset?.key

    const nameBtn = h('button', { class: 'tb-name', 'aria-haspopup': 'menu', onclick: () => this.boardMenu(nameBtn) },
      h('i', { style: { background: b.color } }), b.name, icon('down')) as HTMLButtonElement
    const head = h('div', { class: 'head' },
      h('div', { class: 't' }, h('h1', {}, nameBtn), h('div', { class: 'sub', 'data-live': 'summary' })),
      h('button', { class: this.panelOpen ? 'on' : '', 'aria-pressed': String(this.panelOpen), onclick: () => this.togglePanel() }, icon('chart'), 'Earnings'),
      h('button', { class: 'ic', 'aria-label': 'Board settings', title: 'Board settings', onclick: () => this.editBoard() }, icon('sliders')),
      h('button', { onclick: () => this.openCard(null, v.lists.find(l => !l.isDone)?.id ?? v.lists[0]?.id) }, icon('plus'), 'Add card'))

    this.listsEl = h('div', { class: 'tlists' }, ...v.lists.map((l, i) => this.renderList(l, i)), this.renderAddList())
    const body = h('div', { class: 'tbody' }, this.listsEl, this.panelOpen ? h('aside', { class: 'earn', 'aria-label': 'Monthly earnings', 'data-live': 'panel' }) : null)
    this.el.replaceChildren(head, body)
    this.updateLive()

    if (scroll) {
      this.listsEl.scrollLeft = scroll.left
      this.listsEl.querySelectorAll<HTMLElement>('.tcards').forEach((e, i) => { e.scrollTop = scroll.lists[i] ?? 0 })
    }
    if (focusKey) (this.el.querySelector(`[data-key="${focusKey}"]`) as HTMLElement | null)?.focus()
  }

  private renderList(l: ListView, index: number): HTMLElement {
    const nameBtn = h('button', { class: 'tln', 'aria-haspopup': 'menu', title: 'List options', onclick: () => this.listMenu(nameBtn, l, index) }, l.name,
      l.isDone ? h('span', { class: 'tdone', title: 'Cards here count as done' }, icon('check')) : null) as HTMLButtonElement
    const cards = h('div', { class: 'tcards' }, ...l.cards.map(c => this.renderCard(c, l)))
    if (this.adding === l.id) cards.append(this.renderAdder(l))
    return h('section', { class: 'tlist' + (l.isDone ? ' isdone' : ''), 'data-list': String(l.id), 'aria-label': l.name },
      h('div', { class: 'tlh' }, nameBtn, h('span', { class: 'cnt', 'data-live': 'count' }),
        h('span', { class: 'sp' }), h('span', { class: 'tsum', 'data-live': 'sum' }),
        h('button', { class: 'ic tadd', 'aria-label': `Add a card to ${l.name}`, 'data-key': `add-${l.id}`, onclick: () => { this.adding = l.id; this.render() } }, icon('plus'))),
      cards)
  }

  private renderCard(c: Card, l: ListView): HTMLElement {
    const labels = this.view!.board!.labels.filter(x => c.labels.includes(x.id))
    const today = isoDate(new Date())
    let dueText = '', late = false
    if (c.due) {
      dueText = c.due === today ? 'Today' : c.due === addDays(today, 1) ? 'Tomorrow' : dayFmt.format(new Date(c.due))
      late = c.due < today && !l.isDone
    }
    const m = c.money, s = c.salary
    let pay: HTMLElement | null = null
    if (m && s) {
      const main = s.currency === 'usd' ? m.netUsd : m.netRub
      const other = s.currency === 'usd' ? (m.netRub == null ? '' : fmtRub(m.netRub)) : (m.netUsd == null ? '' : fmtUsd(m.netUsd))
      const tip = `Client pays ${fmtIn(s.currency, s.amount)}, minus ${+m.commission.toFixed(2)}% commission` + (c.paidAt ? '. Paid.' : '')
      pay = h('span', { class: 'tpay', title: tip },
        c.paidAt ? h('span', { class: 'tpaid', role: 'img', 'aria-label': 'Paid' }, icon('check')) : null,
        h('b', {}, fmtIn(s.currency, main ?? s.amount)), other ? h('span', {}, other) : null)
    }
    const checked = c.checklist.filter(i => i.done).length
    const el = h('article', {
      class: 'tcard', tabindex: 0, 'data-card': String(c.id), 'data-key': `card-${c.id}`,
      'aria-label': c.title + (pay ? `, ${pay.textContent}` : ''),
      onpointerdown: (e: PointerEvent) => this.grab(e, c, el),
      onclick: () => { if (!this.drag) this.openCard(c, c.listId) },
      onkeydown: (e: KeyboardEvent) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); this.openCard(c, c.listId) } }
    },
    labels.length ? h('div', { class: 'tlbs', title: labels.map(x => x.name).join(', ') }, labels.map(x => h('i', { style: { background: x.color } }))) : null,
    h('div', { class: 'tt' }, c.title),
    dueText || pay || c.checklist.length
      ? h('div', { class: 'tf' },
        dueText ? h('span', { class: late ? 'late' : '' }, dueText) : null,
        c.checklist.length ? h('span', { class: 'tchk' + (checked === c.checklist.length ? ' all' : '') }, `${checked}/${c.checklist.length}`) : null,
        h('span', { class: 'sp' }), pay)
      : null) as HTMLElement
    return el
  }

  private renderAdder(l: ListView): HTMLElement {
    const inp = h('textarea', { rows: 2, class: 'tadder', placeholder: 'Task title, Enter to add', 'aria-label': `New card in ${l.name}` }) as HTMLTextAreaElement
    const close = () => { this.adding = null; this.render() }
    inp.addEventListener('keydown', async e => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close() }
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault()
        const title = inp.value.trim()
        if (!title) return close()
        try {
          await taskApi.saveCard(l.id, null, { title, description: '', labels: [], due: null, checklist: [], salary: null })
          await this.load()
          ;(this.el.querySelector('.tadder') as HTMLElement | null)?.focus()
        } catch (err) { toast(errText(err)) }
      }
    })
    inp.addEventListener('blur', () => setTimeout(() => { if (this.adding === l.id && !inp.value.trim() && document.activeElement !== inp) close() }, 120))
    setTimeout(() => inp.focus(), 0)
    return inp
  }

  private renderAddList(): HTMLElement {
    if (!this.addingList) {
      return h('button', { class: 'taddlist', 'data-key': 'add-list', onclick: () => { this.addingList = true; this.render() } }, icon('plus'), 'Add list')
    }
    const inp = h('input', { type: 'text', placeholder: 'List name', 'aria-label': 'New list name' }) as HTMLInputElement
    const close = () => { this.addingList = false; this.render() }
    inp.addEventListener('keydown', async e => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close() }
      if (e.key === 'Enter' && inp.value.trim()) {
        try { await taskApi.saveList(this.boardId!, null, inp.value.trim(), false); this.addingList = false; await this.load() } catch (err) { toast(errText(err)) }
      }
    })
    inp.addEventListener('blur', () => setTimeout(() => { if (this.addingList && !inp.value.trim()) close() }, 120))
    setTimeout(() => inp.focus(), 0)
    return h('div', { class: 'tlist tnew' }, inp)
  }

  // ---------- live figures ----------

  /** The board's lists as they look on screen, with any card being dragged in its new place. */
  private effectiveLists(): { list: ListView; cards: Card[] }[] {
    const v = this.view!
    const move = this.drag ?? this.settled
    return v.lists.map(l => {
      let cards = l.cards
      if (move) {
        cards = cards.filter(c => c.id !== move.card.id)
        if (move.over.list === l.id) cards = [...cards.slice(0, move.over.index), move.card, ...cards.slice(move.over.index)]
      }
      return { list: l, cards }
    })
  }

  /** How a drag in progress changes each month's earnings. */
  private monthDelta(): Map<string, Sum> {
    const out = new Map<string, Sum>()
    const move = this.drag ?? this.settled
    if (!move || !move.card.money) return out
    const lists = this.view!.lists
    const from = lists.find(l => l.id === move.fromList)
    const to = lists.find(l => l.id === move.over.list)
    if (!from || !to || from.isDone === to.isDone) return out
    if (to.isDone) out.set(thisMonth(), addMoney(emptySum(), move.card.money, 1))
    else if (move.card.doneAt) out.set(move.card.doneAt.slice(0, 7), addMoney(emptySum(), move.card.money, -1))
    return out
  }

  private updateLive() {
    const v = this.view
    if (!v) return
    const eff = this.effectiveLists()
    const board = emptySum(), pending = emptySum()
    for (const { list, cards } of eff) {
      const s = emptySum()
      for (const c of cards) { addMoney(s, c.money); addMoney(board, c.money); if (!list.isDone) addMoney(pending, c.money) }
      const sec = this.listsEl?.querySelector(`.tlist[data-list="${list.id}"]`)
      if (!sec) continue
      sec.querySelector('[data-live=count]')!.textContent = String(cards.length)
      sec.querySelector('[data-live=sum]')!.textContent = s.n ? fmtSumMain(s) : ''
    }
    const sub = this.el.querySelector('[data-live=summary]')
    if (sub) {
      sub.replaceChildren(...(board.n
        ? ['You get ', h('b', { class: 'okb' }, fmtSumMain(board)), fmtSumOther(board) ? ` (${fmtSumOther(board)})` : '', ' from this board']
        : [eff.reduce((n, x) => n + x.cards.length, 0) ? 'No paid tasks on this board yet. Open a card to add a salary.' : 'Add a card to get started.']))
    }
    const panel = this.el.querySelector<HTMLElement>('[data-live=panel]')
    if (panel) this.drawPanel(panel, pending)
  }

  private drawPanel(panel: HTMLElement, pending: Sum) {
    const v = this.view!
    const delta = this.monthDelta()
    const months = v.months.map(m => {
      const d = delta.get(m.month)
      return { ...m, netUsd: m.netUsd + (d?.usd ?? 0), netRub: m.netRub + (d?.rub ?? 0) }
    })
    const now = months[0]
    if (!now) { panel.replaceChildren(); return }
    const date = (m: string) => new Date(Number(m.slice(0, 4)), Number(m.slice(5, 7)) - 1, 1)
    const nowUsd = Math.max(0, now.netUsd), pend = pending.usdKnown ? pending.usd : 0
    const goal = nowUsd + pend || 1
    const peak = Math.max(nowUsd + pend, ...months.map(m => m.netUsd), 1)
    const pct = (x: number) => `${Math.max(0, Math.min(100, (x / peak) * 100))}%`
    const total = months.reduce((a, m) => ({ usd: a.usd + m.netUsd, rub: a.rub + m.netRub }), { usd: 0, rub: 0 })
    const n = months.length
    panel.replaceChildren(
      h('div', { class: 'ch' }, h('h2', {}, 'Monthly earnings'),
        h('button', { class: 'ic', 'aria-label': 'Close monthly earnings', onclick: () => this.togglePanel() }, icon('x'))),
      h('div', { class: 'hint' }, 'All boards, after commission. A task counts in the month it reached a done list.'),
      h('div', { class: 'earn-now' },
        h('span', { class: 'mute' }, `${monthYearFmt.format(date(now.month))}, so far`),
        h('b', { class: 'big' }, fmtUsd(nowUsd)),
        h('span', {}, fmtRub(Math.max(0, now.netRub))),
        h('div', { class: 'earn-bar', role: 'img', 'aria-label': `${fmtUsd(nowUsd)} earned, ${fmtUsd(pend)} more on this board` },
          h('i', { class: 'got', style: { width: `${(nowUsd / goal) * 100}%` } }), h('i', { class: 'wait', style: { width: `${(pend / goal) * 100}%` } })),
        pend > 0 ? h('span', { class: 'mute' }, `${fmtUsd(pend)} more is still on this board`) : null,
        now.unpaidUsd > 0.005 ? h('span', { class: 'mute' }, `${fmtUsd(now.unpaidUsd)} of it is not paid yet`) : null),
      h('div', { class: 'earn-months' },
        h('span', { class: 'lab' }, 'By month'),
        ...months.map((m, i) => h('div', { class: 'earn-m' + (i === 0 ? ' cur' : '') },
          h('div', { class: 'r' }, h('span', { class: 'nm' }, monthFmt.format(date(m.month))), h('b', {}, fmtUsd(Math.max(0, m.netUsd))), h('span', { class: 'rub' }, fmtRub(Math.max(0, m.netRub)))),
          h('div', { class: 'earn-mbar' }, h('i', { style: { width: pct(Math.max(0, m.netUsd)) } }))))),
      h('div', { class: 'earn-tot' },
        h('div', { class: 'r' }, h('span', { class: 'nm' }, `Last ${n} months`), h('b', {}, fmtUsd(total.usd)), h('span', { class: 'rub' }, fmtRub(total.rub))),
        h('div', { class: 'r' }, h('span', { class: 'nm' }, 'Monthly average'), h('b', {}, fmtUsd(Math.round(total.usd / n))), h('span', { class: 'rub' }, fmtRub(total.rub / n)))))
  }

  private togglePanel() {
    this.panelOpen = !this.panelOpen
    store.set('tasks.earnings', this.panelOpen ? '1' : '0')
    this.render()
  }

  // ---------- menus and dialogs ----------

  private boardMenu(anchor: HTMLElement) {
    const v = this.view!
    const items: MenuItem[] = [{ header: 'Boards' }]
    for (const b of v.boards) {
      items.push({ label: (b.id === v.board?.id ? '✓ ' : '') + b.name, hint: String(b.cards), run: () => { this.boardId = b.id; void this.load() } })
    }
    items.push('sep', { label: 'New board…', run: () => void this.newBoard() }, { label: 'Board settings…', run: () => void this.editBoard() })
    menuAt(anchor, items)
  }

  private async newBoard() {
    const v = this.view!
    const r = await boardDialog(null, v.rate, false, { name: '', color: '#8AA7D9', commission: v.board?.commission ?? 10, labels: v.board?.labels ?? [] })
    if (typeof r === 'number') { this.boardId = r; await this.load() }
  }

  private async editBoard() {
    const v = this.view!
    const r = await boardDialog(v.board, v.rate, v.boards.length > 1)
    if (r === 'deleted') this.boardId = null
    else if (typeof r === 'number') this.boardId = r
    if (r !== undefined) await this.load()
  }

  private async listMenu(anchor: HTMLElement, l: ListView, index: number) {
    const lists = this.view!.lists
    const items: MenuItem[] = [
      { header: l.name },
      { label: 'Rename…', run: async () => {
        const name = await promptBox('Rename list', 'Name', l.name)
        if (name) await this.act(() => taskApi.saveList(this.boardId!, l.id, name, l.isDone))
      } },
      { label: l.isDone ? 'Stop counting as done' : 'Count cards here as done', hint: l.isDone ? '' : 'for earnings', run: () => void this.act(() => taskApi.saveList(this.boardId!, l.id, l.name, !l.isDone)) },
      ...(index > 0 ? [{ label: 'Move left', run: () => void this.act(() => taskApi.moveList(l.id, index - 1)) }] : []),
      ...(index < lists.length - 1 ? [{ label: 'Move right', run: () => void this.act(() => taskApi.moveList(l.id, index + 1)) }] : []),
      'sep',
      { label: 'Delete list…', danger: true, run: async () => {
        if (l.cards.length && !(await confirmBox('Delete this list?', `“${l.name}” and its ${l.cards.length} ${l.cards.length === 1 ? 'card' : 'cards'} will be removed.`))) return
        await this.act(() => taskApi.deleteList(l.id))
      } }
    ]
    menuAt(anchor, items)
  }

  private async act(f: () => Promise<unknown>) {
    try { await f(); await this.load() } catch (e) { toast(errText(e)) }
  }

  private async openCard(c: Card | null, listId: number | undefined) {
    const v = this.view
    if (!v?.board || listId == null) return
    const changed = await cardDialog({ board: v.board, lists: v.lists, listId, card: c, rate: v.rate })
    if (changed) await this.load()
  }

  // ---------- dragging ----------

  private grab(e: PointerEvent, card: Card, el: HTMLElement) {
    if (e.button !== 0 || this.drag || (e.target as HTMLElement).closest('button,a,input,textarea')) return
    primeSound()
    this.pending = { card, el, x: e.clientX, y: e.clientY }
    this.ptr = { x: e.clientX, y: e.clientY }
  }

  private move(e: PointerEvent) {
    this.ptr = { x: e.clientX, y: e.clientY }
    const p = this.pending
    if (!p || this.drag) return
    if (Math.hypot(e.clientX - p.x, e.clientY - p.y) < 5) return
    this.pending = null
    if (!p.el.isConnected) return
    const r = p.el.getBoundingClientRect()
    const list = this.view!.lists.find(l => l.id === p.card.listId)!
    const fromIndex = list.cards.findIndex(c => c.id === p.card.id)
    const ghost = p.el.cloneNode(true) as HTMLElement
    ghost.className = 'tcard tghost'
    ghost.removeAttribute('tabindex')
    ghost.setAttribute('aria-hidden', 'true')
    Object.assign(ghost.style, { width: r.width + 'px', left: r.left + 'px', top: r.top + 'px', transformOrigin: `${p.x - r.left}px ${p.y - r.top}px` })
    document.body.append(ghost)
    const slot = h('div', { class: 'tslot', style: { height: r.height + 'px' } })
    p.el.replaceWith(slot)
    document.body.classList.add('tdragging')
    this.drag = {
      card: p.card, el: p.el, ghost, slot, fromList: list.id, fromIndex,
      offX: p.x - r.left, offY: p.y - r.top, w: r.width, h: r.height,
      tilt: 0, vx: 0, lastX: e.clientX, lift: 0, over: { list: list.id, index: fromIndex }, dropping: false
    }
    this.highlight()
    playClick('detach', e.clientX)
    const step = () => { if (!this.drag || this.drag.dropping) return; this.frame(); this.raf = requestAnimationFrame(step) }
    this.raf = requestAnimationFrame(step)
  }

  private frame() {
    const d = this.drag!, p = this.ptr
    const dx = p.x - d.lastX
    d.lastX = p.x
    d.vx = d.vx * 0.7 + dx * 0.3
    d.tilt += (Math.max(-10, Math.min(10, d.vx * 1.1)) - d.tilt) * 0.2
    d.lift += (1 - d.lift) * 0.25
    const l = d.lift
    Object.assign(d.ghost.style, {
      left: (p.x - d.offX) + 'px', top: (p.y - d.offY) + 'px',
      transform: `rotate(${d.tilt.toFixed(2)}deg) scale(${(1 + 0.045 * l).toFixed(4)})`,
      boxShadow: `0 ${(2 + 14 * l).toFixed(1)}px ${(6 + 26 * l).toFixed(1)}px rgba(0,0,0,${(0.3 + 0.25 * l).toFixed(2)})`
    })
    this.autoScroll()
    const over = this.hit(p.x - d.offX + d.w / 2, p.y - d.offY + d.h / 2)
    if (over && (over.list !== d.over.list || over.index !== d.over.index)) this.placeSlot(over)
  }

  /** Scrolls the lists sideways, and a long list up or down, when the card nears an edge. */
  private autoScroll() {
    const box = this.listsEl
    if (!box) return
    const p = this.ptr, edge = 70
    const r = box.getBoundingClientRect()
    const speed = (d: number) => Math.round(((edge - d) / edge) * 18)
    if (p.x < r.left + edge) box.scrollLeft -= speed(p.x - r.left)
    else if (p.x > r.right - edge) box.scrollLeft += speed(r.right - p.x)
    const cards = this.listsEl!.querySelector<HTMLElement>(`.tlist[data-list="${this.drag!.over.list}"] .tcards`)
    if (cards && cards.scrollHeight > cards.clientHeight) {
      const c = cards.getBoundingClientRect()
      if (p.y < c.top + 40) cards.scrollTop -= 10
      else if (p.y > c.bottom - 40) cards.scrollTop += 10
    }
  }

  private hit(cx: number, cy: number): { list: number; index: number } | null {
    let best: HTMLElement | null = null, bestDist = Infinity
    for (const el of this.listsEl!.querySelectorAll<HTMLElement>('.tlist[data-list]')) {
      const r = el.getBoundingClientRect()
      const dist = cx < r.left ? r.left - cx : cx > r.right ? cx - r.right : 0
      if (dist < bestDist) { bestDist = dist; best = el }
    }
    if (!best || bestDist > 160) return null
    let index = 0
    for (const el of best.querySelectorAll<HTMLElement>('.tcard')) {
      const r = el.getBoundingClientRect()
      if (cy > r.top + r.height / 2) index++
    }
    return { list: Number(best.dataset.list), index }
  }

  /** Moves the empty slot, letting the other cards slide out of the way. */
  private placeSlot(over: { list: number; index: number }) {
    const d = this.drag!
    const before = new Map<Element, DOMRect>()
    this.listsEl!.querySelectorAll('.tcard').forEach(el => before.set(el, el.getBoundingClientRect()))
    const cards = this.listsEl!.querySelector<HTMLElement>(`.tlist[data-list="${over.list}"] .tcards`)
    if (!cards) return
    const others = [...cards.querySelectorAll<HTMLElement>(':scope > .tcard')]
    cards.insertBefore(d.slot, others[over.index] ?? cards.querySelector(':scope > .tadder'))
    d.over = over
    for (const [el, a] of before) {
      const e = el as HTMLElement
      e.style.transition = 'none'
      e.style.transform = ''
      const b = e.getBoundingClientRect()
      const dx = a.left - b.left, dy = a.top - b.top
      if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) continue
      e.style.transform = `translate(${dx}px, ${dy}px)`
      e.getBoundingClientRect()
      e.style.transition = 'transform .22s cubic-bezier(.2,.8,.2,1)'
      e.style.transform = ''
    }
    this.highlight()
    this.updateLive()
  }

  private highlight() {
    const d = this.drag
    this.listsEl?.querySelectorAll<HTMLElement>('.tlist').forEach(el => el.classList.toggle('over', !!d && Number(el.dataset.list) === d.over.list))
  }

  private up() {
    if (this.pending && !this.drag) {
      this.pending = null
      if (this.reloadAfterDrag) { this.reloadAfterDrag = false; setTimeout(() => void this.load(), 0) }
      return
    }
    const d = this.drag
    if (!d || d.dropping) return
    this.drop()
  }

  private cancelDrag() {
    const d = this.drag
    if (!d || d.dropping) return
    this.placeSlot({ list: d.fromList, index: d.fromIndex })
    this.drop()
  }

  /** Settles the card into its slot: it glides in, clicks, and gives a small bounce. */
  private drop() {
    const d = this.drag!
    d.dropping = true
    cancelAnimationFrame(this.raf)
    this.eatClick = true
    setTimeout(() => { this.eatClick = false }, 80)
    const r = d.slot.getBoundingClientRect()
    const ease = 'cubic-bezier(.25,.9,.3,1)'
    Object.assign(d.ghost.style, {
      transition: `left .17s ${ease}, top .17s ${ease}, transform .17s ${ease}, box-shadow .17s ease`,
      left: r.left + 'px', top: r.top + 'px', transform: 'none', boxShadow: '0 1px 2px rgba(0,0,0,.3)'
    })
    setTimeout(() => playClick('attach', r.left + d.w / 2), 115)
    setTimeout(() => this.finish(d), 175)
  }

  private finish(d: Drag) {
    d.slot.replaceWith(d.el)
    d.ghost.remove()
    document.body.classList.remove('tdragging')
    d.el.classList.add('landed')
    setTimeout(() => d.el.classList.remove('landed'), 400)
    const moved = d.over.list !== d.fromList || d.over.index !== d.fromIndex
    this.drag = null
    this.highlight()
    if (!moved) { this.updateLive(); if (this.reloadAfterDrag) { this.reloadAfterDrag = false; void this.load() } return }
    // Show the result straight away; the stored figures follow.
    this.settled = d
    this.updateLive()
    taskApi.moveCard(d.card.id, d.over.list, d.over.index)
      .then(() => setTimeout(() => void this.load(), 420))
      .catch(e => { toast(errText(e)); this.settled = null; void this.load() })
    this.reloadAfterDrag = false
  }
}
