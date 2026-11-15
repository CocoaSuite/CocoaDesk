import { api, errText, type Period, type Semester, type SemesterInput, type WeekKind } from '../api'
import { h, fmtDayMonth } from '../util'
import { modal, field, select, confirmBox, toast } from './dialogs'
import { icon } from './icons'

/** A sensible first guess for a new semester: the current half of the academic year. */
export function defaultSemester(today = new Date()): SemesterInput {
  const y = today.getFullYear(), m = today.getMonth() + 1
  if (m >= 8 || m === 1) {
    const year = m === 1 ? y - 1 : y
    return { name: `Autumn ${year}`, start: `${year}-09-01`, end: `${year}-12-31`, firstWeek: 'top' }
  }
  return { name: `Spring ${y}`, start: `${y}-02-09`, end: `${y}-06-30`, firstWeek: 'top' }
}

export function semesterFields(v: SemesterInput) {
  const name = h('input', { type: 'text', value: v.name, maxlength: 80 }) as HTMLInputElement
  const start = h('input', { type: 'date', value: v.start }) as HTMLInputElement
  const end = h('input', { type: 'date', value: v.end }) as HTMLInputElement
  const first = select<WeekKind>([['top', 'Top week'], ['bottom', 'Bottom week']], v.firstWeek)
  const el = h('div', { style: 'display:flex;flex-direction:column;gap:10px' },
    field('Name', name),
    h('div', { class: 'g3' }, field('Starts', start), field('Ends', end), field('The first week is a', first)))
  return {
    el,
    value: (): SemesterInput => ({ name: name.value.trim(), start: start.value, end: end.value, firstWeek: first.value as WeekKind })
  }
}

/**
 * Creates or edits a semester with its bell schedule and breaks.
 * Resolves to the semester id when saved, `null` when deleted, `undefined` when cancelled.
 */
export async function semesterDialog(sem: Semester | null): Promise<number | null | undefined> {
  const base = sem ? { name: sem.name, start: sem.start, end: sem.end, firstWeek: sem.firstWeek } : defaultSemester()
  const f = semesterFields(base)
  const err = h('div', { class: 'err', role: 'alert' })

  // Bell schedule
  let periods: Period[] = sem?.periods.map(p => ({ ...p })) ?? []
  const plist = h('div', { style: 'display:flex;flex-direction:column;gap:6px' })
  const drawPeriods = () => {
    plist.replaceChildren(
      periods.length ? h('div', { class: 'pl hint' }, h('span', {}, '№'), h('span', {}, 'Starts'), h('span', {}, 'Ends'), h('span')) : h('div', { class: 'hint' }, 'No periods yet.'),
      ...periods.map((p, i) => {
        const s = h('input', { type: 'time', value: p.start, 'aria-label': `Period ${p.number} start`, onchange: () => (p.start = s.value) }) as HTMLInputElement
        const e = h('input', { type: 'time', value: p.end, 'aria-label': `Period ${p.number} end`, onchange: () => (p.end = e.value) }) as HTMLInputElement
        return h('div', { class: 'pl' }, h('span', { class: 'n' }, String(p.number)), s, e,
          h('button', { class: 'ic', 'aria-label': `Remove period ${p.number}`, onclick: () => { periods.splice(i, 1); drawPeriods() } }, icon('x')))
      }))
  }
  drawPeriods()
  const addPeriod = () => {
    const last = periods[periods.length - 1]
    const next = (last?.number ?? 0) + 1
    const shift = (t: string, mins: number) => {
      const [hh, mm] = t.split(':').map(Number)
      const total = Math.min(23 * 60 + 59, hh * 60 + mm + mins)
      return `${String(Math.floor(total / 60)).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}`
    }
    const start = last ? shift(last.end, 10) : '09:00'
    periods.push({ number: next, start, end: shift(start, 80) })
    drawPeriods()
  }

  // Breaks (only once the semester exists)
  const blist = h('div', { class: 'blist' })
  const drawBreaks = () => {
    if (!sem) { blist.replaceChildren(h('div', { class: 'hint' }, 'You can add holidays and breaks after saving.')); return }
    blist.replaceChildren(...(sem.breaks.length ? sem.breaks.map(b => h('div', {},
      h('span', {}, `${b.label || 'Break'} · ${b.from === b.to ? fmtDayMonth(b.from) : `${fmtDayMonth(b.from)} – ${fmtDayMonth(b.to)}`}`),
      h('button', { class: 'ic', 'aria-label': `Remove ${b.label || 'break'}`, onclick: async () => {
        try { await api.deleteBreak(b.id); sem.breaks = sem.breaks.filter(x => x.id !== b.id); drawBreaks() } catch (e) { err.textContent = errText(e) }
      } }, icon('x')))) : [h('div', { class: 'hint' }, 'No breaks.')]))
  }
  drawBreaks()
  const bFrom = h('input', { type: 'date', 'aria-label': 'Break starts' }) as HTMLInputElement
  const bTo = h('input', { type: 'date', 'aria-label': 'Break ends (leave empty for one day)' }) as HTMLInputElement
  const bLabel = h('input', { type: 'text', placeholder: 'e.g. Holiday', 'aria-label': 'Break name', maxlength: 60 }) as HTMLInputElement
  const addBreak = async () => {
    if (!sem) return
    if (!bFrom.value) { err.textContent = 'Pick the first day of the break.'; return }
    try {
      const to = bTo.value || bFrom.value
      const id = await api.addBreak(sem.id, bFrom.value, to, bLabel.value)
      sem.breaks.push({ id, from: bFrom.value, to, label: bLabel.value.trim() })
      sem.breaks.sort((a, b) => a.from.localeCompare(b.from))
      bFrom.value = bTo.value = bLabel.value = ''
      err.textContent = ''
      drawBreaks()
    } catch (e) { err.textContent = errText(e) }
  }

  const save = async () => {
    const v = f.value()
    if (!v.name) { err.textContent = 'Give the semester a name.'; return }
    if (!v.start || !v.end) { err.textContent = 'Pick the start and end dates.'; return }
    if (periods.some(p => !p.start || !p.end)) { err.textContent = 'Every period needs a start and end time.'; return }
    try {
      const id = await api.saveSemester(sem?.id ?? null, v)
      await api.setPeriods(id, periods)
      toast(sem ? 'Semester saved' : 'Semester created')
      m.close(id)
    } catch (e) { err.textContent = errText(e) }
  }
  const del = async () => {
    if (!sem) return
    const n = sem.classCount
    if (!(await confirmBox('Delete semester?', `“${sem.name}”, its ${n} ${n === 1 ? 'class' : 'classes'} and their history will be removed.`))) return
    try { await api.deleteSemester(sem.id); toast('Semester deleted'); m.close(null) } catch (e) { err.textContent = errText(e) }
  }

  const m = modal<number | null>(sem ? 'Semester' : 'New semester', [
    f.el,
    h('div', { class: 'hint' }, 'Weeks alternate from the week the semester starts in. Breaks don’t change the count.'),
    h('div', { class: 'lab', style: 'margin-top:6px' }, 'Class times'),
    plist,
    h('button', { style: 'align-self:flex-start', onclick: addPeriod }, icon('plus'), 'Add period'),
    h('div', { class: 'lab', style: 'margin-top:6px' }, 'Holidays and breaks'),
    blist,
    sem ? h('div', { class: 'g3', style: 'grid-template-columns:1fr 1fr 1.3fr auto;align-items:end' },
      field('From', bFrom), field('To', bTo), field('Name', bLabel), h('button', { onclick: addBreak }, 'Add')) : null,
    err,
    h('div', { class: 'row' },
      sem ? h('button', { class: 'danger', onclick: del }, 'Delete semester') : null,
      h('span', { class: 'sp' }),
      h('button', { onclick: () => m.close(undefined) }, 'Cancel'),
      h('button', { class: 'pri', onclick: save }, sem ? 'Save' : 'Create semester'))
  ].filter(Boolean) as Node[], { size: 'wide' })
  return m.done
}
