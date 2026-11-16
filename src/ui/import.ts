import { api, errText, KIND_LABEL, type ImportDone, type ImportItem, type ImportPreview, type Semester, type ImportTarget } from '../api'
import { h, WEEKDAYS } from '../util'
import { modal, field, select, toast } from './dialogs'
import { defaultSemester, semesterFields } from './semester'

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December']

/** Picks an Excel file, shows what will be imported and applies it. */
export async function importFlow(semesters: Semester[], currentId: number | null): Promise<ImportDone | null> {
  let picked
  try { picked = await api.pickTimetable() } catch (e) { toast(errText(e)); return null }
  if (!picked) return null
  const file = picked

  let preview: ImportPreview
  try { preview = await api.importPreview(file.token, null, null) } catch (e) { toast(errText(e), 5000); return null }

  let targetId: number | 'new' = currentId ?? semesters[semesters.length - 1]?.id ?? 'new'
  if (!semesters.length) targetId = 'new'
  const included = new Set<string>()
  const err = h('div', { class: 'err', role: 'alert' })

  const sheetPick = preview.sheets.length > 1
    ? select<string>(preview.sheets.map(s => [s, s]), preview.sheet, { onchange: () => reload(sheetPick!.value, null) })
    : null
  const groupPick = h('select') as HTMLSelectElement
  groupPick.addEventListener('change', () => reload(preview.sheet, groupPick.value))
  const targetPick = select<string>([...semesters.map(s => [String(s.id), s.name] as [string, string]), ['new', 'New semester…']], String(targetId),
    { onchange: () => { targetId = targetPick.value === 'new' ? 'new' : Number(targetPick.value); void syncTarget() } })
  const newSem = semesterFields(defaultSemester())
  const newWrap = h('div', {}, newSem.el)
  const useTimes = h('input', { type: 'checkbox' }) as HTMLInputElement
  const timesLabel = h('span')
  const tableWrap = h('div', { class: 'itable' })
  const probs = h('div', { class: 'probs', role: 'status' })
  const count = h('span', { class: 'hint' })
  const go = h('button', { class: 'pri' }) as HTMLButtonElement

  const drawGroups = () => {
    groupPick.replaceChildren(...preview.groups.map(g => h('option', { value: g.label, selected: g.label === preview.group }, g.detail ? `${g.label} — ${g.detail}` : g.label)))
  }

  const describeMonths = (it: ImportItem) => it.months ? (it.months[0] === it.months[1] ? `${MONTHS[it.months[0] - 1]} only` : `${MONTHS[it.months[0] - 1]} – ${MONTHS[it.months[1] - 1]} only`) : ''

  const drawTable = () => {
    const rows: Node[] = []
    let day = 0
    for (const it of preview.items) {
      if (it.weekday !== day) {
        day = it.weekday
        rows.push(h('tr', { class: 'day' }, h('td', { colspan: 6 }, WEEKDAYS[day - 1])))
      }
      const cb = h('input', { type: 'checkbox', checked: included.has(it.key), 'aria-label': `Import ${it.subject}, ${WEEKDAYS[it.weekday - 1]} period ${it.period}` }) as HTMLInputElement
      const tr = h('tr', { class: included.has(it.key) ? '' : 'off' },
        h('td', { class: 'c' }, cb),
        h('td', {}, String(it.period)),
        h('td', {}, it.weeks === 'every' ? h('span', { class: 'chip' }, 'Every week') : h('span', { class: 'chip sel' }, it.weeks === 'top' ? 'Top week' : 'Bottom week')),
        h('td', {}, it.subject, h('span', { class: 'sub' }, [KIND_LABEL[it.kind], it.shared ? 'whole stream' : ''].filter(Boolean).join(', '))),
        h('td', {}, it.room || '—', it.teacher ? h('span', { class: 'sub' }, it.teacher) : null),
        h('td', {}, [describeMonths(it), it.note].filter(Boolean).join('; ') || ''))
      cb.addEventListener('change', () => {
        cb.checked ? included.add(it.key) : included.delete(it.key)
        tr.className = cb.checked ? '' : 'off'
        drawCount()
      })
      rows.push(tr)
    }
    tableWrap.replaceChildren(h('table', {},
      h('thead', {}, h('tr', {}, h('th', {}, h('span', { class: 'sr' }, 'Import')), h('th', {}, '№'), h('th', {}, 'Weeks'), h('th', {}, 'Class'), h('th', {}, 'Room / teacher'), h('th', {}, 'Notes'))),
      h('tbody', {}, ...rows)))
    probs.replaceChildren(...preview.problems.map(p => h('div', {}, p)))
    probs.hidden = !preview.problems.length
    const n = preview.periods.length
    timesLabel.textContent = n ? `Use the class times from the file (${n} periods, ${preview.periods[0].start}–${preview.periods[n - 1].end})` : 'The file has no class times'
    useTimes.disabled = !n
  }

  const drawCount = () => {
    const n = included.size
    count.textContent = `${n} of ${preview.items.length} classes selected`
    go.textContent = n ? `Import ${n} ${n === 1 ? 'class' : 'classes'}` : 'Import'
    go.disabled = n === 0
  }

  /** Ticks every class except the ones unticked last time for this semester. */
  const syncTarget = async () => {
    newWrap.hidden = targetId !== 'new'
    let excluded: string[] = []
    if (targetId !== 'new') {
      try { excluded = await api.importExclusions(targetId) } catch { excluded = [] }
    }
    included.clear()
    for (const it of preview.items) if (!excluded.includes(it.key)) included.add(it.key)
    const sem = targetId === 'new' ? null : semesters.find(s => s.id === targetId)
    useTimes.checked = !sem || !sem.periods.length || differs(sem)
    drawTable()
    drawCount()
  }
  const differs = (sem: Semester) =>
    sem.periods.length !== preview.periods.length || sem.periods.some((p, i) => p.start !== preview.periods[i]?.start || p.end !== preview.periods[i]?.end)

  const reload = async (sheet: string, group: string | null) => {
    try {
      preview = await api.importPreview(file.token, sheet, group)
      err.textContent = ''
      drawGroups()
      await syncTarget()
    } catch (e) { err.textContent = errText(e) }
  }

  go.addEventListener('click', async () => {
    let target: ImportTarget
    if (targetId === 'new') {
      const s = newSem.value()
      if (!s.name || !s.start || !s.end) { err.textContent = 'Fill in the new semester’s name and dates.'; return }
      target = { kind: 'new', semester: s }
    } else target = { kind: 'existing', id: targetId }
    go.disabled = true
    try {
      const done = await api.importCommit(file.token, {
        sheet: preview.sheet, group: preview.group, target, includedKeys: [...included], useFilePeriods: useTimes.checked
      })
      const parts = [done.added && `${done.added} added`, done.updated && `${done.updated} updated`, done.removed && `${done.removed} removed`].filter(Boolean)
      toast(parts.length ? `Imported: ${parts.join(', ')}` : 'Nothing changed')
      m.close(done)
    } catch (e) {
      err.textContent = errText(e)
      go.disabled = false
    }
  })

  drawGroups()
  const m = modal<ImportDone>(`Import “${file.name}”`, [
    h('div', { class: 'g2' },
      sheetPick ? field('Sheet', sheetPick) : null,
      field('Your group', groupPick),
      field('Add to', targetPick)),
    newWrap,
    h('label', { class: 'sw', style: 'gap:8px' }, useTimes, timesLabel),
    probs,
    tableWrap,
    h('div', { class: 'hint' }, 'Untick anything that isn’t yours, like the other stream of a language class. CocoaDesk remembers your choices for the next import, and importing a newer version of the file updates classes in place.'),
    err,
    h('div', { class: 'row' }, count, h('span', { class: 'sp' }), h('button', { onclick: () => m.close(undefined) }, 'Cancel'), go)
  ].filter(Boolean) as Node[], { size: 'xwide' })
  await syncTarget()
  return (await m.done) ?? null
}
