import { api, errText, KIND_LABEL, type Class, type ClassInput, type ClassKind, type Semester, type Weeks } from '../api'
import { h, WEEKDAYS } from '../util'
import { modal, field, select, confirmBox, toast } from './dialogs'

/** Adds or edits a class. Resolves to true when something was saved or deleted. */
export async function classDialog(sem: Semester, c: Class | null, preset?: { weekday: number; period: number }): Promise<boolean> {
  const v: ClassInput = c
    ? { subject: c.subject, kind: c.kind, teacher: c.teacher, room: c.room, note: c.note, weekday: c.weekday, period: c.period, weeks: c.weeks, validFrom: c.validFrom, validTo: c.validTo }
    : { subject: '', kind: 'seminar', teacher: '', room: '', note: '', weekday: preset?.weekday ?? 1, period: preset?.period ?? sem.periods[0]?.number ?? 1, weeks: 'every', validFrom: null, validTo: null }

  const subject = h('input', { type: 'text', value: v.subject, maxlength: 200 }) as HTMLInputElement
  const kind = select<ClassKind>((Object.keys(KIND_LABEL) as ClassKind[]).map(k => [k, KIND_LABEL[k]]), v.kind)
  const day = select<number>(WEEKDAYS.map((d, i) => [i + 1, d]), v.weekday)
  const periodOpts: [number, string][] = sem.periods.map(p => [p.number, `${p.number}: ${p.start}–${p.end}`])
  if (!sem.periods.some(p => p.number === v.period)) periodOpts.push([v.period, `${v.period}: no time set`])
  const period = select<number>(periodOpts, v.period)
  const weeks = select<Weeks>([['every', 'Every week'], ['top', 'Top weeks only'], ['bottom', 'Bottom weeks only']], v.weeks)
  const teacher = h('input', { type: 'text', value: v.teacher, maxlength: 200 }) as HTMLInputElement
  const room = h('input', { type: 'text', value: v.room, maxlength: 100 }) as HTMLInputElement
  const note = h('input', { type: 'text', value: v.note, maxlength: 500 }) as HTMLInputElement
  const from = h('input', { type: 'date', value: v.validFrom ?? '', min: sem.start, max: sem.end }) as HTMLInputElement
  const to = h('input', { type: 'date', value: v.validTo ?? '', min: sem.start, max: sem.end }) as HTMLInputElement
  const err = h('div', { class: 'err', role: 'alert' })

  const save = async () => {
    const input: ClassInput = {
      subject: subject.value.trim(), kind: kind.value as ClassKind, teacher: teacher.value, room: room.value, note: note.value,
      weekday: Number(day.value), period: Number(period.value), weeks: weeks.value as Weeks,
      validFrom: from.value || null, validTo: to.value || null
    }
    if (!input.subject) { err.textContent = 'A class needs a subject.'; subject.focus(); return }
    try {
      await api.saveClass(sem.id, c?.id ?? null, input)
      toast(c ? 'Class updated' : 'Class added')
      m.close(true)
    } catch (e) { err.textContent = errText(e) }
  }
  const del = async () => {
    if (!c) return
    if (!(await confirmBox('Delete class?', `“${c.subject}” and its history will be removed.`))) return
    try { await api.deleteClass(c.id); toast('Class deleted'); m.close(true) } catch (e) { err.textContent = errText(e) }
  }
  subject.addEventListener('keydown', e => { if (e.key === 'Enter') void save() })

  const m = modal<boolean>(c ? 'Edit class' : 'Add class', [
    field('Subject', subject),
    h('div', { class: 'g2' }, field('Type', kind), field('Runs', weeks)),
    h('div', { class: 'g2' }, field('Day', day), field('Period', period)),
    h('div', { class: 'g2' }, field('Room', room), field('Teacher', teacher)),
    field('Note', note),
    h('div', { class: 'g2' }, field('Only from (optional)', from), field('Until (optional)', to)),
    c?.source === 'import' ? h('div', { class: 'hint' }, 'This class came from an import. If you import a newer version of the file, its details are updated from the file.') : null,
    err,
    h('div', { class: 'row' },
      c ? h('button', { class: 'danger', onclick: del }, 'Delete') : null,
      h('span', { class: 'sp' }),
      h('button', { onclick: () => m.close(false) }, 'Cancel'),
      h('button', { class: 'pri', onclick: save }, c ? 'Save' : 'Add class'))
  ].filter(Boolean) as Node[], { size: 'wide' })
  return (await m.done) ?? false
}
