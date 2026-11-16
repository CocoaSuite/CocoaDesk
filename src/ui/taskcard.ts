import { taskApi, errText, type Card, type CardInput, type ChecklistItem, type Currency, type ListView, type RateInfo, type Salary, type TaskBoard } from '../api'
import { h } from '../util'
import { icon } from './icons'
import { modal, confirmBox, toast } from './dialogs'
import { money, fmtUsd, fmtRub } from './money'

const dayFmt = new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short' })

/** "1 $ = 81.23 ₽, Central Bank rate for 16 Nov" */
export function rateText(rate: RateInfo | null): string {
  if (!rate) return 'No exchange rate yet. The other currency shows once CocoaDesk has been online.'
  const v = `1 $ = ${rate.value.toFixed(2)} ₽`
  if (rate.manual) return `${v}, your own rate`
  return rate.date ? `${v}, Central Bank rate for ${dayFmt.format(new Date(rate.date))}` : v
}

/** Reads "1 200,50" or "1200.5". */
export function parseAmount(s: string): number | null {
  const t = s.replace(/[\s  $₽]/g, '').replace(',', '.')
  if (!t) return null
  const v = Number(t)
  return Number.isFinite(v) ? v : NaN
}

export interface CardDialogOpts {
  board: TaskBoard
  lists: ListView[]
  listId: number
  card: Card | null
  rate: RateInfo | null
}

/** Opens a card for editing, or a new one. Resolves to true when something changed. */
export function cardDialog(o: CardDialogOpts): Promise<boolean> {
  const c = o.card
  let changed = false
  const title = h('input', { type: 'text', class: 'tc-title', value: c?.title ?? '', placeholder: 'What needs doing?', 'aria-label': 'Title' }) as HTMLInputElement

  // where and when
  const listSel = h('select', { 'aria-label': 'List' }, ...o.lists.map(l => h('option', { value: String(l.id), selected: l.id === o.listId }, l.name))) as HTMLSelectElement
  const due = h('input', { type: 'date', value: c?.due ?? '', 'aria-label': 'Due date' }) as HTMLInputElement

  // labels
  const chosen = new Set(c?.labels ?? [])
  const labelBtns = o.board.labels.map(l => {
    const b = h('button', { type: 'button', class: 'tc-lab', 'aria-pressed': String(chosen.has(l.id)), style: `--c:${l.color}` },
      h('i'), l.name) as HTMLButtonElement
    b.onclick = () => {
      chosen.has(l.id) ? chosen.delete(l.id) : chosen.add(l.id)
      b.setAttribute('aria-pressed', String(chosen.has(l.id)))
    }
    return b
  })

  const desc = h('textarea', { rows: 4, placeholder: 'Details, links, what the client asked for…', 'aria-label': 'Description' }, c?.description ?? '') as HTMLTextAreaElement

  // checklist
  const items: ChecklistItem[] = (c?.checklist ?? []).map(i => ({ ...i }))
  const checkList = h('div', { class: 'tc-check' })
  const progress = h('span', { class: 'mute' })
  const bar = h('i')
  const addItem = h('input', { type: 'text', placeholder: 'Add an item and press Enter', 'aria-label': 'New checklist item' }) as HTMLInputElement
  const drawChecklist = () => {
    const done = items.filter(i => i.done).length
    progress.textContent = items.length ? `${done} of ${items.length}` : ''
    bar.style.width = items.length ? `${(done / items.length) * 100}%` : '0'
    checkList.replaceChildren(...items.map((it, i) => {
      const box = h('input', { type: 'checkbox', checked: it.done, 'aria-label': 'Done', onchange: () => { it.done = box.checked; drawChecklist() } }) as HTMLInputElement
      const text = h('input', { type: 'text', value: it.text, class: it.done ? 'done' : '', 'aria-label': 'Item', oninput: () => { it.text = text.value } }) as HTMLInputElement
      return h('div', { class: 'tc-item' }, box, text,
        h('button', { class: 'ic', type: 'button', 'aria-label': 'Remove item', onclick: () => { items.splice(i, 1); drawChecklist() } }, icon('x')))
    }))
  }
  addItem.addEventListener('keydown', e => {
    if (e.key === 'Enter' && addItem.value.trim()) {
      e.preventDefault()
      items.push({ text: addItem.value.trim(), done: false })
      addItem.value = ''
      drawChecklist()
    }
  })
  drawChecklist()

  // salary
  const s0 = c?.salary ?? null
  let on = !!s0
  let currency: Currency = s0?.currency ?? 'usd'
  const onBox = h('input', { type: 'checkbox', checked: on }) as HTMLInputElement
  const amount = h('input', { type: 'text', inputmode: 'decimal', class: 'tc-amount', value: s0 ? String(s0.amount) : '', placeholder: '0', 'aria-label': 'Amount the client pays' }) as HTMLInputElement
  const curBtns = (['usd', 'rub'] as Currency[]).map(k =>
    h('button', { type: 'button', 'aria-pressed': String(k === currency), class: k === currency ? 'on' : '', onclick: () => { currency = k; drawPay() } }, k === 'usd' ? '$ USD' : '₽ RUB') as HTMLButtonElement)
  const comm = h('input', { type: 'text', inputmode: 'decimal', class: 'tc-comm', value: s0?.commission != null ? String(s0.commission) : '', placeholder: String(o.board.commission), 'aria-label': 'Commission percent' }) as HTMLInputElement
  const breakdown = h('div', { class: 'tc-break' })
  const payFields = h('div', { class: 'tc-payf' })
  const paidBox = h('input', { type: 'checkbox', checked: !!c?.paidAt }) as HTMLInputElement
  const paidNote = h('span', { class: 'mute' })
  const paidRow = h('label', { class: 'tc-paid' }, paidBox, h('span', {}, 'Payment received', paidNote))

  const salary = (): Salary | null | 'bad' => {
    if (!on) return null
    const a = parseAmount(amount.value)
    if (a == null || !(a > 0)) return 'bad'
    const cm = comm.value.trim() ? parseAmount(comm.value) : null
    if (cm != null && (Number.isNaN(cm) || cm < 0 || cm > 100)) return 'bad'
    return { amount: a, currency, commission: cm }
  }

  const drawPay = () => {
    curBtns.forEach((b, i) => { const k = i ? 'rub' : 'usd'; b.classList.toggle('on', k === currency); b.setAttribute('aria-pressed', String(k === currency)) })
    payFields.hidden = !on
    const s = salary()
    const rate = c?.paidAt && c.lockedRate ? c.lockedRate : o.rate?.value ?? null
    if (!on || s === 'bad' || !s) {
      breakdown.replaceChildren(h('div', { class: 'mute' }, on ? 'Enter the amount the client pays.' : ''))
    } else {
      const m = money(s, o.board.commission, rate)
      const u = (v: number | null, neg = false) => h('span', { class: 'n' }, v == null ? '' : (neg ? '−' : '') + fmtUsd(v))
      const r = (v: number | null, neg = false) => h('span', { class: 'n' }, v == null ? '' : (neg ? '−' : '') + fmtRub(v))
      const cut = (v: number | null) => (v == null ? null : v * m.commission / 100)
      breakdown.replaceChildren(
        h('div', { class: 'tc-brow hd' }, h('span'), h('span', { class: 'n' }, 'USD'), h('span', { class: 'n' }, 'RUB')),
        h('div', { class: 'tc-brow' }, h('span', {}, 'Client pays'), u(m.grossUsd), r(m.grossRub)),
        h('div', { class: 'tc-brow' }, h('span', {}, `Commission ${+m.commission.toFixed(2)}%`), u(cut(m.grossUsd), true), r(cut(m.grossRub), true)),
        h('div', { class: 'tc-brow get' }, h('span', {}, 'You get'), u(m.netUsd), r(m.netRub)))
    }
    paidRow.hidden = !c || !c.salary || !on
    paidNote.textContent = c?.paidAt
      ? ` on ${dayFmt.format(new Date(c.paidAt))}${c.lockedRate ? `, at 1 $ = ${c.lockedRate.toFixed(2)} ₽` : ''}`
      : ''
  }
  onBox.onchange = () => { on = onBox.checked; drawPay(); if (on) setTimeout(() => amount.focus(), 0) }
  amount.oninput = drawPay
  comm.oninput = drawPay
  paidBox.onchange = async () => {
    if (!c) return
    try {
      await taskApi.setPaid(c.id, paidBox.checked)
      changed = true
      c.paidAt = paidBox.checked ? new Date().toISOString().slice(0, 19) : null
      c.lockedRate = paidBox.checked ? o.rate?.value ?? null : null
      drawPay()
    } catch (e) { paidBox.checked = !paidBox.checked; toast(errText(e)) }
  }
  drawPay()

  const err = h('div', { class: 'err', role: 'alert' })
  const save = async () => {
    err.textContent = ''
    const s = salary()
    if (!title.value.trim()) { err.textContent = 'Give the task a title.'; title.focus(); return }
    if (s === 'bad') { err.textContent = 'Check the amount and the commission (0–100%).'; amount.focus(); return }
    const input: CardInput = {
      title: title.value.trim(), description: desc.value, labels: o.board.labels.map(l => l.id).filter(id => chosen.has(id)),
      due: due.value || null, checklist: items.filter(i => i.text.trim()), salary: s
    }
    try {
      const id = await taskApi.saveCard(o.listId, c?.id ?? null, input)
      const target = Number(listSel.value)
      if (target !== o.listId) {
        const n = o.lists.find(l => l.id === target)?.cards.length ?? 0
        await taskApi.moveCard(id, target, n)
      }
      m.close(true)
    } catch (e) { err.textContent = errText(e) }
  }

  const del = c ? h('button', { class: 'danger', onclick: async () => {
    if (!(await confirmBox('Delete this task?', `“${c.title}” will be removed for good.`))) return
    try { await taskApi.deleteCard(c.id); m.close(true) } catch (e) { toast(errText(e)) }
  } }, icon('trash'), 'Delete') : null

  const m = modal<boolean>(c ? 'Task' : 'New task', [
    h('div', { class: 'tc' },
      h('div', { class: 'tc-main' },
        title,
        h('div', { class: 'tc-meta' },
          h('label', { class: 'f' }, 'List', listSel),
          h('label', { class: 'f' }, 'Due', due)),
        o.board.labels.length ? h('div', { class: 'tc-sec' }, h('span', { class: 'lab' }, 'Labels'), h('div', { class: 'tc-labs' }, labelBtns)) : null,
        h('div', { class: 'tc-sec' }, h('span', { class: 'lab' }, 'Description'), desc),
        h('div', { class: 'tc-sec' },
          h('div', { class: 'ch' }, h('span', { class: 'lab' }, 'Checklist'), progress),
          h('div', { class: 'tc-prog' }, bar),
          checkList, addItem)),
      h('aside', { class: 'tc-pay', 'aria-label': 'Salary' },
        h('label', { class: 'sw' }, h('span', { class: 'tg' }, onBox, h('i')),
          h('span', { class: 'tx' }, 'Salary', h('span', {}, 'Leave off for tasks you don’t get paid for'))),
        payFields)),
    err,
    h('div', { class: 'row' }, del, h('span', { class: 'sp' }),
      h('button', { onclick: () => m.close(changed) }, 'Cancel'),
      h('button', { class: 'pri', onclick: save }, 'Save'))
  ], { size: 'xwide' })
  m.box.querySelector('h2')?.remove()
  payFields.append(
    h('label', { class: 'f' }, 'Amount the client pays', h('div', { class: 'tc-amt' }, amount, h('div', { class: 'seg', role: 'group', 'aria-label': 'Currency' }, curBtns))),
    h('label', { class: 'f tc-commf' }, 'Service commission', h('div', { class: 'tc-pct' }, comm, '%')),
    h('div', { class: 'hint' }, `Leave empty to use the board’s ${+o.board.commission.toFixed(2)}%.`),
    breakdown,
    h('div', { class: 'hint tc-rate' }, icon('refresh'), rateText(o.rate)),
    paidRow)
  drawPay()
  m.box.addEventListener('keydown', e => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); void save() } })
  if (!c) setTimeout(() => title.focus(), 30)
  return m.done.then(v => !!v || changed)
}
