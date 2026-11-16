import { taskApi, errText, type BoardInput, type Label, type RateInfo, type TaskBoard } from '../api'
import { h } from '../util'
import { icon } from './icons'
import { modal, confirmBox, toast, field, toggle } from './dialogs'
import { rateText, parseAmount } from './taskcard'
import { soundPrefs, setSoundPrefs, primeSound, playClick } from './sound'

export const BOARD_COLORS = ['#8AA7D9', '#c9a24a', '#5b9f74', '#b48fd6', '#e0735d', '#7aa9b3']
const LABEL_COLORS = ['#6f8fc4', '#a07fc2', '#c9a24a', '#5b9f74', '#e0735d', '#7aa9b3', '#d68fb0', '#9a9a9a']

/** Board settings: name, colour, commission, labels, the exchange rate and drag sounds.
 *  Resolves to the board's id when saved, `'deleted'` when removed. */
export function boardDialog(board: TaskBoard | null, rate: RateInfo | null, canDelete: boolean, template?: BoardInput): Promise<number | 'deleted' | undefined> {
  const base: BoardInput | null = board ?? template ?? null
  const name = h('input', { type: 'text', value: base?.name ?? '', placeholder: 'Freelance, Study, Home…' }) as HTMLInputElement
  let color = base?.color ?? BOARD_COLORS[0]
  const swatches = h('div', { class: 'swatches', role: 'radiogroup', 'aria-label': 'Colour' })
  const drawSw = () => swatches.replaceChildren(...BOARD_COLORS.map(c =>
    h('button', { type: 'button', role: 'radio', 'aria-checked': String(c === color), 'aria-label': c, class: 'sw-c' + (c === color ? ' on' : ''), style: `--c:${c}`, onclick: () => { color = c; drawSw() } })))
  drawSw()
  const commission = h('input', { type: 'text', inputmode: 'decimal', value: String(base?.commission ?? 10), class: 'tc-comm' }) as HTMLInputElement

  // labels
  const labels: Label[] = (base?.labels ?? []).map(l => ({ ...l }))
  const labelBox = h('div', { class: 'tb-labels' })
  const drawLabels = () => labelBox.replaceChildren(...labels.map((l, i) => {
    const dot = h('button', { type: 'button', class: 'sw-c', style: `--c:${l.color}`, 'aria-label': 'Change colour', onclick: () => {
      l.color = LABEL_COLORS[(LABEL_COLORS.indexOf(l.color) + 1) % LABEL_COLORS.length]; drawLabels()
    } })
    const inp = h('input', { type: 'text', value: l.name, 'aria-label': 'Label name', oninput: () => { l.name = inp.value } }) as HTMLInputElement
    return h('div', { class: 'tb-label' }, dot, inp, h('button', { class: 'ic', type: 'button', 'aria-label': 'Remove label', onclick: () => { labels.splice(i, 1); drawLabels() } }, icon('x')))
  }), h('button', { type: 'button', class: 'link', onclick: () => {
    labels.push({ id: 'l' + Date.now().toString(36), name: 'New label', color: LABEL_COLORS[labels.length % LABEL_COLORS.length] }); drawLabels()
    ;(labelBox.querySelectorAll('input')[labels.length - 1] as HTMLInputElement | undefined)?.select()
  } }, 'Add a label'))
  drawLabels()

  // exchange rate (shared by every board)
  const rateLine = h('div', { class: 'hint' }, rateText(rate))
  const manual = h('input', { type: 'text', inputmode: 'decimal', value: rate?.manual ? String(rate.value) : '', placeholder: 'e.g. 80', class: 'tc-comm', 'aria-label': 'Your own rate' }) as HTMLInputElement
  const manualRow = h('div', { class: 'tc-pct' }, '1 $ =', manual, '₽')
  manualRow.hidden = !rate?.manual
  const useManual = toggle('Use my own rate', 'Instead of the Central Bank’s', !!rate?.manual, v => { manualRow.hidden = !v; if (v) manual.focus() })
  const refresh = h('button', { type: 'button', onclick: async () => {
    refresh.disabled = true
    try { const r = await taskApi.refreshRate(); rateLine.textContent = rateText(r) } catch (e) { toast(errText(e)) } finally { refresh.disabled = false }
  } }, icon('refresh'), 'Update now') as HTMLButtonElement

  // drag sounds (this computer)
  const sp = soundPrefs()
  const vol = h('input', { type: 'range', min: '0', max: '1', step: '0.05', value: String(sp.volume), 'aria-label': 'Volume', class: 'tb-vol' }) as HTMLInputElement
  vol.oninput = () => setSoundPrefs({ ...soundPrefs(), volume: Number(vol.value) })
  vol.onchange = () => { primeSound(); setTimeout(() => playClick('attach'), 60) }
  const sounds = toggle('Sounds when dragging cards', 'A click when a card lifts off and when it lands', sp.on, v => setSoundPrefs({ ...soundPrefs(), on: v }))

  const err = h('div', { class: 'err', role: 'alert' })
  const save = async () => {
    err.textContent = ''
    const cm = parseAmount(commission.value)
    if (cm == null || Number.isNaN(cm) || cm < 0 || cm > 100) { err.textContent = 'The commission has to be between 0 and 100%.'; commission.focus(); return }
    const useOwn = (useManual.querySelector('input') as HTMLInputElement).checked
    const own = parseAmount(manual.value)
    if (useOwn && (own == null || Number.isNaN(own) || own <= 0)) { err.textContent = 'Enter your own rate, or switch it off.'; manual.focus(); return }
    const input: BoardInput = { name: name.value, color, commission: cm, labels: labels.filter(l => l.name.trim()).map(l => ({ ...l, name: l.name.trim() })) }
    try {
      const id = await taskApi.saveBoard(board?.id ?? null, input)
      if (useOwn !== !!rate?.manual || (useOwn && own !== rate?.value)) await taskApi.setManualRate(useOwn ? own : null)
      m.close(id)
    } catch (e) { err.textContent = errText(e) }
  }
  const del = board && canDelete ? h('button', { class: 'danger', onclick: async () => {
    if (!(await confirmBox('Delete this board?', `“${board.name}” and all its lists and cards will be removed. Their earnings leave the monthly totals too.`))) return
    try { await taskApi.deleteBoard(board.id); m.close('deleted') } catch (e) { toast(errText(e)) }
  } }, icon('trash'), 'Delete board') : null

  const m = modal<number | 'deleted'>(board ? 'Board settings' : 'New board', [
    field('Name', name),
    h('div', { class: 'f-row' }, h('span', { class: 'mute' }, 'Colour'), swatches),
    h('label', { class: 'f' }, 'Service commission', h('div', { class: 'tc-pct' }, commission, '%')),
    h('div', { class: 'hint' }, 'Taken off every salary on this board, unless a task sets its own.'),
    h('div', { class: 'tb-sec' }, h('span', { class: 'lab' }, 'Labels'), labelBox),
    h('div', { class: 'tb-sec' }, h('span', { class: 'lab' }, 'Dollar rate'), rateLine,
      h('div', { class: 'row', style: { justifyContent: 'flex-start', marginTop: '0' } }, refresh), useManual, manualRow,
      h('div', { class: 'hint' }, 'Used by every board. A task marked paid keeps the rate of that day.')),
    h('div', { class: 'tb-sec' }, h('span', { class: 'lab' }, 'This computer'), sounds, h('label', { class: 'f-row' }, h('span', { class: 'mute' }, 'Volume'), vol)),
    err,
    h('div', { class: 'row' }, del, h('span', { class: 'sp' }), h('button', { onclick: () => m.close(undefined) }, 'Cancel'), h('button', { class: 'pri', onclick: save }, board ? 'Save' : 'Create board'))
  ], { size: 'wide' })
  name.addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); void save() } })
  return m.done
}
