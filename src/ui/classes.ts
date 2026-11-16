import { api, errText, KIND_LABEL, type Day, type Entry, type Class, type WeekView, type Settings, type MarkStatus } from '../api'
import { h, addDays, fmtRange, fmtDayMonth, fmtWeekday, fmtMinutes, WD_SHORT } from '../util'
import { icon } from './icons'
import { toast, menuAt, confirmBox, toggle, select, type MenuItem } from './dialogs'
import { classDialog } from './classdialog'
import { semesterDialog } from './semester'
import { importFlow } from './import'

interface State { semesterId: number | null; date: string | null; view: WeekView | null }

const KIND_VAR = (k: string) => `var(--k-${k})`

export class ClassesScreen {
  el = h('main', { 'aria-label': 'Classes' })
  private st: State = { semesterId: null, date: null, view: null }
  private loading: Promise<void> | null = null
  private onStatus: (text: string, cls?: string) => void

  constructor(onStatus: (text: string, cls?: string) => void) {
    this.onStatus = onStatus
  }

  /** Reloads from the backend and redraws. Calls made while loading collapse into one. */
  async load(): Promise<void> {
    if (this.loading) return this.loading
    this.loading = (async () => {
      try {
        const v = await api.weekView(this.st.semesterId, this.st.date)
        this.st.view = v
        if (v.semester) this.st.semesterId = v.semester.id
        this.render()
      } catch (e) {
        this.el.replaceChildren(h('div', { class: 'fatal' }, 'Could not load your classes: ' + errText(e)))
      } finally {
        this.loading = null
      }
    })()
    return this.loading
  }

  private go(date: string | null) { this.st.date = date; void this.load() }

  private render() {
    const v = this.st.view!
    const focus = document.activeElement as HTMLElement | null
    const focusKey = focus?.dataset?.key
    this.el.replaceChildren(...(v.semester ? this.renderWeek(v) : this.renderEmpty(v)))
    if (focusKey) (this.el.querySelector(`[data-key="${CSS.escape(focusKey)}"]`) as HTMLElement | null)?.focus()
    this.updateStatus(v)
  }

  private updateStatus(v: WeekView) {
    const now = v.today?.entries.find(e => e.status === 'inProgress')
    if (now) this.onStatus(`In class: ${now.class.subject}, ${fmtMinutes(now.minutesLeft ?? 0)} left`, 'live')
    else if (v.semesters.length) this.onStatus(v.settings.autoMark ? 'Marking classes automatically' : 'Automatic marking is off')
    else this.onStatus('')
  }

  private renderEmpty(v: WeekView): Node[] {
    return [
      h('div', { class: 'empty' },
        h('h2', {}, v.semesters.length ? 'No semester here' : 'Set up your timetable'),
        h('div', {}, 'Import your university’s Excel timetable, or set up a semester and add your classes by hand. Either way you can edit everything afterwards.'),
        h('div', { class: 'row' },
          h('button', { class: 'pri', onclick: () => this.doImport() }, icon('import'), 'Import from Excel'),
          h('button', { onclick: () => this.editSemester(null) }, icon('plus'), 'Set up a semester by hand')))
    ]
  }

  private renderWeek(v: WeekView): Node[] {
    const sem = v.semester!
    const kindWord = v.weekKind === 'top' ? 'top week' : v.weekKind === 'bottom' ? 'bottom week' : null
    const sub = h('div', { class: 'sub' }, sem.name,
      v.weekNumber && v.weekNumber > 0 && v.weekCount ? `, week ${v.weekNumber} of ${v.weekCount}` : ', outside the semester',
      kindWord ? [', ', h('b', {}, kindWord)] : null)

    const semPick = v.semesters.length > 1
      ? select(v.semesters.map(s => [s.id, s.name] as [number, string]), sem.id, {
        'aria-label': 'Semester', style: 'width:auto',
        onchange: (e: Event) => { this.st.semesterId = Number((e.target as HTMLSelectElement).value); this.st.date = null; void this.load() }
      })
      : null

    const head = h('div', { class: 'head' },
      h('div', { class: 't' }, h('h1', {}, 'Classes'), sub),
      semPick,
      h('button', { onclick: () => this.doImport() }, icon('import'), 'Import'),
      h('button', { onclick: () => this.editSemester(sem.id) }, icon('sliders'), 'Semester'),
      h('button', { class: 'pri', onclick: () => this.editClass(null, { weekday: todayWeekday(v), period: sem.periods[0]?.number ?? 1 }) }, icon('plus'), 'Add class'))

    const sunday = addDays(v.monday, 6)
    const thisWeek = v.days.some(d => d.isToday)
    const bar = h('div', { class: 'bar' },
      h('button', { class: '', 'aria-label': 'Previous week', onclick: () => this.go(addDays(v.monday, -7)) }, icon('left')),
      h('button', { 'aria-label': 'Next week', onclick: () => this.go(addDays(v.monday, 7)) }, icon('right')),
      h('span', { class: 'when' }, fmtRange(v.monday, sunday)),
      thisWeek ? null : h('button', { onclick: () => this.go(null) }, 'This week'))

    return [head, bar, h('div', { class: 'cols' }, this.grid(v), this.side(v))]
  }

  private grid(v: WeekView): HTMLElement {
    const sem = v.semester!
    const hasSunday = v.days[6].entries.length > 0 || v.days[6].otherWeek.length > 0 || sem.periods.length === 0
    const days = v.days.slice(0, hasSunday ? 7 : 6)
    const week = h('div', { class: 'week', style: `--days:${days.length}`, role: 'table', 'aria-label': 'Week timetable' })
    week.append(h('div', { class: 'wrow hd', role: 'row' },
      h('div', { role: 'columnheader' }, h('span', { class: 'sr' }, 'Period')),
      ...days.map(d => h('div', { class: 'dh' + (d.isToday ? ' col-today' : ''), role: 'columnheader' },
        h('b', {}, WD_SHORT[d.weekday - 1]), h('span', { class: 'd' }, fmtDayMonth(d.date)),
        d.isToday ? h('span', { class: 'today' }, 'Today') : null))))

    if (!sem.periods.length) {
      week.append(h('div', { class: 'daynote', style: 'padding:16px' }, 'This semester has no bell schedule yet. ',
        h('button', { class: 'link', onclick: () => this.editSemester(sem.id) }, 'Add the class times'), ' to start adding classes.'))
    }
    for (const p of sem.periods) {
      const row = h('div', { class: 'wrow', role: 'row' },
        h('div', { class: 'pn', role: 'rowheader' }, h('b', {}, String(p.number)), h('span', {}, p.start), h('span', {}, p.end)))
      for (const d of days) row.append(this.slot(d, p.number))
      week.append(row)
    }
    const kinds = [...new Set(days.flatMap(d => d.entries.map(e => e.class.kind).concat(d.otherWeek.map(c => c.kind))))]
    const legend = h('div', { class: 'legend' },
      ...kinds.map(k => h('span', {}, h('i', { style: `--k:${KIND_VAR(k)}` }), KIND_LABEL[k])),
      days.some(d => d.otherWeek.length) ? h('span', {}, h('i', { class: 'dash' }), v.weekKind === 'top' ? 'Bottom week — not this week' : 'Top week — not this week') : null,
      h('span', {}, icon('check', 'i chk'), v.settings.autoMark ? 'Marked done when it ended' : 'Done'))
    return h('section', { class: 'card weekwrap', 'aria-label': 'Timetable' }, week, legend)
  }

  private slot(d: Day, period: number): HTMLElement {
    const entries = d.entries.filter(e => e.class.period === period)
    const ghosts = d.otherWeek.filter(c => c.period === period)
    const usable = d.inSemester && !d.breakLabel
    const cell = h('div', { class: 'slot' + (d.isToday ? ' col-today' : '') + (usable ? '' : ' off'), role: 'cell' })
    if (!usable && period === 1) cell.append(h('div', { class: 'daynote' }, d.breakLabel || (d.inSemester ? '' : 'Outside the semester')))
    for (const e of entries) cell.append(this.entryCard(e))
    for (const g of ghosts) {
      cell.append(h('button', {
        class: 'ghost', 'data-key': `g${g.id}${d.date}`,
        'aria-label': `${g.subject}, ${g.weeks} week only, not this week`,
        onclick: () => this.editClass(g)
      }, h('span', {}, g.weeks === 'top' ? 'Top week only' : 'Bottom week only'), h('b', {}, `${g.subject} (${KIND_LABEL[g.kind].toLowerCase()})`)))
    }
    if (usable && !entries.length) {
      cell.classList.add('add')
      cell.title = 'Add a class here'
      cell.addEventListener('click', ev => { if (ev.target === cell) this.editClass(null, { weekday: d.weekday, period }) })
    }
    return cell
  }

  private entryCard(e: Entry): HTMLElement {
    const c = e.class
    const st = e.status
    const cls = st === 'inProgress' ? 'now' : st
    const where = [c.room, c.teacher].filter(Boolean).join(', ')
    const card = h('button', {
      class: 'cls ' + cls, style: `--k:${KIND_VAR(c.kind)}`, 'data-key': `e${c.id}${e.date}`,
      'aria-label': `${c.subject}, ${KIND_LABEL[c.kind]}, ${e.start} to ${e.end}. ${statusText(e)}`,
      onclick: (ev: Event) => this.entryMenu(ev.currentTarget as HTMLElement, e)
    },
    h('i'),
    h('span', { class: 'k' }, h('span', {}, KIND_LABEL[c.kind]),
      c.weeks !== 'every' ? h('span', { class: 'chip sel', style: 'font-size:10px;padding:0 5px' }, c.weeks === 'top' ? 'Top' : 'Bottom') : null,
      st === 'done' ? icon('check', 'i chk') : null),
    h('span', { class: 's', lang: langOf(c.subject) }, c.subject),
    where ? h('span', { class: 'w' }, where) : null)
    if (st === 'inProgress') {
      card.append(h('span', { class: 'prog' }, h('i', { style: `width:${Math.round((e.progress ?? 0) * 100)}%` })),
        h('span', { class: 'now-t' }, `Now, ${fmtMinutes(e.minutesLeft ?? 0)} left`))
    } else if (st === 'skipped') card.append(h('span', { class: 'n', style: 'color:var(--warnfg)' }, 'Skipped'))
    else if (st === 'cancelled') card.append(h('span', { class: 'n', style: 'color:var(--bad)' }, 'Cancelled'))
    else if (c.note) card.append(h('span', { class: 'n mute' }, c.note))
    return card
  }

  private entryMenu(anchor: HTMLElement, e: Entry) {
    const set = (s: MarkStatus | null, msg: string) => async () => {
      try { await api.setStatus(e.class.id, e.date, s); toast(msg); await this.load() } catch (err) { toast(errText(err)) }
    }
    const items: MenuItem[] = [{ header: `${e.class.subject}, ${fmtWeekday(e.date)} ${e.start}–${e.end}` }]
    const past = ['done', 'skipped', 'ended', 'untracked'].includes(e.status) || (e.status === 'cancelled' && e.minutesUntil == null)
    if (e.status === 'cancelled') items.push({ label: 'Not cancelled after all', run: set(null, 'Class restored') })
    else if (past) {
      if (e.status !== 'done') items.push({ label: 'Mark as done', run: set('done', 'Marked as done') })
      if (e.status !== 'skipped') items.push({ label: 'I missed it', run: set('skipped', 'Marked as skipped') })
      items.push({ label: 'It was cancelled', run: set('cancelled', 'Marked as cancelled') })
      if (e.status === 'done' || e.status === 'skipped') items.push({ label: 'Clear', run: set(null, 'Cleared') })
    } else {
      items.push({ label: 'Cancel this time only', run: set('cancelled', 'Cancelled — it won’t be reminded or marked') })
      if (e.status === 'inProgress') items.push({ label: 'I’m not there', run: set('skipped', 'Marked as skipped') })
    }
    items.push('sep', { label: 'Edit class…', run: () => this.editClass(e.class) })
    items.push({ label: 'Delete class…', danger: true, run: () => this.deleteClass(e.class) })
    menuAt(anchor, items)
  }

  private side(v: WeekView): HTMLElement {
    const col = h('div', { class: 'side' })
    if (v.today) col.append(this.todayCard(v.today))
    col.append(this.weeksCard(v), this.settingsCard(v.settings))
    return col
  }

  private todayCard(d: Day): HTMLElement {
    const card = h('section', { class: 'card cardpad', 'aria-label': 'Today' },
      h('div', { class: 'ch' }, h('span', { class: 'lab' }, `Today, ${fmtWeekday(d.date)}`)))
    if (d.breakLabel) card.append(h('div', { class: 'hint' }, `No classes — ${d.breakLabel}.`))
    else if (!d.entries.length) card.append(h('div', { class: 'hint' }, 'No classes today.'))
    for (const e of d.entries) {
      const st = statusText(e)
      const tone = e.status === 'done' ? 'ok' : e.status === 'inProgress' ? 'sel' : e.status === 'skipped' || e.status === 'cancelled' ? 'warn' : ''
      card.append(h('div', { class: 'trow' },
        h('span', { class: 'dot ' + (e.status === 'inProgress' ? 'now' : e.status) }, e.status === 'done' ? icon('check') : null),
        h('span', { class: 'b' },
          h('b', {}, e.class.subject),
          h('span', {}, [KIND_LABEL[e.class.kind], e.class.room].filter(Boolean).join(', ')),
          h('span', { class: 'st ' + tone }, st)),
        h('span', { class: 'tm' }, e.start)))
    }
    return card
  }

  private weeksCard(v: WeekView): HTMLElement {
    const sem = v.semester!
    const card = h('section', { class: 'card cardpad', 'aria-label': 'Alternating weeks' }, h('span', { class: 'lab' }, 'Alternating weeks'))
    if (v.weekKind) {
      const next = v.weekKind === 'top' ? 'bottom' : 'top'
      const thisWeek = v.days.some(d => d.isToday)
      const box = (kind: string, label: string, on: boolean) =>
        h('div', { class: on ? 'on' : '' }, h('b', {}, kind === 'top' ? 'Top week' : 'Bottom week'), h('span', {}, label))
      card.append(h('div', { class: 'wk' },
        box(v.weekKind, `${thisWeek ? 'This week' : 'Shown'}, ${fmtRange(v.monday, addDays(v.monday, 6))}`, true),
        box(next, `Next, ${fmtRange(addDays(v.monday, 7), addDays(v.monday, 13))}`, false)))
    }
    card.append(h('div', { class: 'hint' },
      `Counting from the week of ${fmtDayMonth(sem.start)}, which is a ${sem.firstWeek} week. Breaks don’t change the count.`),
    h('button', { class: 'link', style: 'align-self:flex-start;font-size:12px', onclick: () => this.editSemester(sem.id) }, 'Change semester, breaks or class times'))
    return card
  }

  private settingsCard(s: Settings): HTMLElement {
    const save = async (patch: Partial<Settings>) => {
      try { await api.saveSettings({ ...s, ...patch }); await this.load() } catch (e) { toast(errText(e)) }
    }
    const remind = select<number>([[0, 'Off'], [5, '5 minutes before'], [10, '10 minutes before'], [15, '15 minutes before'], [30, '30 minutes before'], [60, '1 hour before']],
      [0, 5, 10, 15, 30, 60].includes(s.remindMinutes) ? s.remindMinutes : 10,
      { id: 'remind', onchange: (e: Event) => save({ remindMinutes: Number((e.target as HTMLSelectElement).value) }) })
    return h('section', { class: 'card cardpad', 'aria-label': 'When classes happen' },
      h('span', { class: 'lab' }, 'When classes happen'),
      toggle('Mark classes done when they end', 'Change it from the class if you missed it', s.autoMark, v => save({ autoMark: v })),
      h('div', { class: 'fr' }, h('label', { for: 'remind' }, 'Remind me'), h('div', { class: 'grow' }, remind)),
      toggle('Keep running when the window is closed', 'So marking and reminders carry on', s.closeToTray, v => save({ closeToTray: v })))
  }

  private async editClass(c: Class | null, preset?: { weekday: number; period: number }) {
    const v = this.st.view!
    const sem = v.semester
    if (!sem) return
    if (!sem.periods.length) { toast('Add the class times to the semester first.'); return this.editSemester(sem.id) }
    const saved = await classDialog(sem, c, preset)
    if (saved) await this.load()
  }

  private async deleteClass(c: Class) {
    const extra = c.source === 'import' ? ' It came from an import, so it will come back if you import the same file again unless you untick it there.' : ''
    if (!(await confirmBox('Delete class?', `“${c.subject}” and its history will be removed.${extra}`))) return
    try { await api.deleteClass(c.id); toast('Class deleted'); await this.load() } catch (e) { toast(errText(e)) }
  }

  private async editSemester(id: number | null) {
    const v = this.st.view!
    const sem = id == null ? null : v.semesters.find(s => s.id === id) ?? null
    const res = await semesterDialog(sem)
    if (res === undefined) return
    this.st.semesterId = res
    if (res == null) this.st.date = null
    await this.load()
  }

  private async doImport() {
    const v = this.st.view
    const done = await importFlow(v?.semesters ?? [], this.st.semesterId)
    if (done) {
      this.st.semesterId = done.semesterId
      this.st.date = null
      await this.load()
    }
  }
}

/** Lets the browser hyphenate long Russian subject names properly. */
const langOf = (s: string) => (/[а-яё]/i.test(s) ? 'ru' : undefined)

function todayWeekday(v: WeekView): number {
  const t = v.days.find(d => d.isToday)
  return t ? Math.min(t.weekday, 6) : 1
}

export function statusText(e: Entry): string {
  switch (e.status) {
    case 'done': return e.markedBy === 'auto' ? `Done automatically at ${e.end}` : 'Marked as done'
    case 'skipped': return 'Skipped'
    case 'cancelled': return 'Cancelled'
    case 'inProgress': return `In progress, ends ${e.end}, ${fmtMinutes(e.minutesLeft ?? 0)} left`
    case 'ended': return 'Ended, not marked'
    case 'untracked': return 'Before it was added'
    case 'upcoming': {
      const m = e.minutesUntil ?? 0
      return m < 12 * 60 ? `Starts in ${fmtMinutes(m)}` : `${fmtWeekday(e.date)} at ${e.start}`
    }
  }
}
