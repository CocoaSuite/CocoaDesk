import {
  researchApi, newsApi, errText, ALL_SOURCES, SOURCE_LABEL, SOURCE_HINT,
  type ResearchQuery, type ResearchTopic, type ResearchTopicInput, type ResearchSettings, type PaperSource, type KindFilter, type SortBy, type CiteStyle
} from '../api'
import { h } from '../util'
import { modal, field, select, confirmBox, toast, toggle } from './dialogs'
import { icon } from './icons'

const split = (s: string) => s.split(/[,\n;]/).map(x => x.trim()).filter(Boolean)
const join = (v: string[]) => v.join(', ')
const num = (s: string) => { const n = Number(s); return s.trim() && Number.isFinite(n) ? Math.round(n) : null }

export const blankQuery = (sources: PaperSource[] = ALL_SOURCES): ResearchQuery => ({
  text: '', fromYear: null, toYear: null, openAccess: false, minCitations: null, kind: 'any', language: null,
  sources: [...sources], exclude: [], blocked: [], sort: 'relevance'
})

export const SORT_NAME: Record<SortBy, string> = { relevance: 'Best match', newest: 'Newest first', mostCited: 'Most cited' }
export const STYLE_NAME: Record<CiteStyle, string> = { gost: 'GOST', apa: 'APA', bibtex: 'BibTeX' }

/** Filter controls shared by the search bar and the topic dialog. `read()` returns the filters as set. */
export function filterFields(q: ResearchQuery, opts: { sort?: boolean } = {}) {
  const from = h('input', { type: 'number', min: 1800, max: 2100, value: q.fromYear ?? '', placeholder: 'Any', 'aria-label': 'From year' }) as HTMLInputElement
  const to = h('input', { type: 'number', min: 1800, max: 2100, value: q.toYear ?? '', placeholder: 'Any', 'aria-label': 'To year' }) as HTMLInputElement
  const minCit = h('input', { type: 'number', min: 0, value: q.minCitations ?? '', placeholder: 'Any' }) as HTMLInputElement
  const kind = select<KindFilter>([['any', 'Any kind'], ['article', 'Journal articles'], ['preprint', 'Preprints'], ['review', 'Reviews']], q.kind)
  const lang = select<string>([['', 'Any language'], ['ru', 'Russian'], ['en', 'English']], q.language ?? '')
  const sort = select<SortBy>((Object.keys(SORT_NAME) as SortBy[]).map(s => [s, SORT_NAME[s]]), q.sort)
  const exclude = h('input', { type: 'text', value: join(q.exclude), placeholder: 'обзор, survey' }) as HTMLInputElement
  let open = q.openAccess
  const els: Node[] = [
    h('div', { class: 'g2' },
      h('div', { class: 'f', style: 'display:flex;flex-direction:column;gap:4px;font-size:12px;color:var(--mute)' }, 'Years',
        h('div', { class: 'inl' }, from, h('span', { class: 'mute' }, '–'), to)),
      field('At least this many citations', minCit)),
    h('div', { class: 'g2' }, field('Kind', kind), field('Language', lang)),
    h('div', { class: 'g2' }, field('Leave out papers that mention', exclude), opts.sort ? field('Order', sort) : h('span')),
    toggle('Free to read only', 'Only papers with an open full text or PDF', open, v => (open = v))
  ]
  const read = (): Omit<ResearchQuery, 'text' | 'sources' | 'blocked'> => {
    let a = num(from.value), b = num(to.value)
    if (a != null && b != null && a > b) [a, b] = [b, a]
    const mc = num(minCit.value)
    return {
      fromYear: a, toYear: b, openAccess: open, minCitations: mc != null && mc > 0 ? mc : null,
      kind: kind.value as KindFilter, language: (lang.value || null) as 'ru' | 'en' | null, exclude: split(exclude.value), sort: sort.value as SortBy
    }
  }
  return { els, read }
}

/** Checkboxes for the databases to ask. */
export function sourcePicker(chosen: PaperSource[]) {
  const boxes = ALL_SOURCES.map(s => [s, h('input', { type: 'checkbox', checked: chosen.includes(s) }) as HTMLInputElement] as const)
  const el = h('div', { class: 'srcpick' }, ...boxes.map(([s, b]) => h('label', {}, b, h('span', {}, h('b', {}, SOURCE_LABEL[s]), h('span', {}, SOURCE_HINT[s])))))
  return { el, read: () => boxes.filter(([, b]) => b.checked).map(([s]) => s) }
}

function splitInterval(m: number): [number, number] {
  return m % 1440 === 0 ? [m / 1440, 1440] : [Math.max(1, Math.round(m / 60)), 60]
}

/**
 * Creates or edits a watched research topic. `start` prefills a new topic (from "Watch this search").
 * Resolves to the topic id when saved, `null` when deleted, `undefined` when cancelled.
 */
export async function researchTopicDialog(t: ResearchTopic | null, start: ResearchQuery): Promise<number | null | undefined> {
  const v: ResearchTopicInput = t ? { ...t } : { ...start, name: start.text.slice(0, 80), enabled: true, intervalMinutes: 1440, notify: true }
  const err = h('div', { class: 'err', role: 'alert' })
  const name = h('input', { type: 'text', value: v.name, maxlength: 80, placeholder: 'Нейросети в диагностике' }) as HTMLInputElement
  const text = h('input', { type: 'text', value: v.text, placeholder: 'нейронные сети диагностика пневмонии' }) as HTMLInputElement
  const filters = filterFields(v)
  const sources = sourcePicker(v.sources)
  const blocked = h('input', { type: 'text', value: join(v.blocked), placeholder: 'Journal name or website' }) as HTMLInputElement
  const [n, unit] = splitInterval(v.intervalMinutes)
  const every = h('input', { type: 'number', min: 1, max: 999, value: n, 'aria-label': 'Check every', style: 'width:80px;flex:none' }) as HTMLInputElement
  const everyUnit = select<number>([[60, 'hours'], [1440, 'days']], unit, { 'aria-label': 'Unit' })
  let notify = v.notify, enabled = v.enabled

  const save = async () => {
    const f = filters.read()
    const input: ResearchTopicInput = {
      ...f, sort: 'newest', name: name.value.trim() || text.value.trim().slice(0, 80), text: text.value.trim(),
      sources: sources.read(), blocked: split(blocked.value), enabled, notify,
      intervalMinutes: Math.max(60, Math.round(Math.max(1, Number(every.value) || 0) * Number(everyUnit.value)))
    }
    if (!input.text) { err.textContent = 'Type what to look for.'; text.focus(); return }
    if (!input.sources.length) { err.textContent = 'Choose at least one database.'; return }
    try {
      const id = await researchApi.saveTopic(t?.id ?? null, input)
      toast(t ? 'Topic saved, checking now' : 'Topic added, checking now')
      m.close(id)
    } catch (e) { err.textContent = errText(e) }
  }
  const del = async () => {
    if (!t) return
    if (!(await confirmBox('Delete topic?', `“${t.name}” and the papers it found will be removed. Papers in your library stay.`))) return
    try { await researchApi.deleteTopic(t.id); toast('Topic deleted'); m.close(null) } catch (e) { err.textContent = errText(e) }
  }

  const m = modal<number | null>(t ? 'Edit topic' : 'Watch a topic', [
    field('Name', name),
    field('Look for', text),
    h('div', { class: 'hint' }, 'New papers that match are collected on the schedule below, newest first.'),
    h('div', { class: 'lab', style: 'margin-top:4px' }, 'Filters'),
    ...filters.els,
    h('div', { class: 'lab', style: 'margin-top:4px' }, 'Databases'),
    sources.el,
    field('Leave out these journals or websites (this topic)', blocked),
    h('div', { class: 'lab', style: 'margin-top:4px' }, 'When'),
    h('div', { class: 'f', style: 'display:flex;flex-direction:column;gap:4px;font-size:12px;color:var(--mute)' }, 'Check every', h('div', { class: 'inl', style: 'max-width:260px' }, every, everyUnit)),
    toggle('Notify me', 'A notification when new papers are found', notify, x => (notify = x)),
    toggle('Checking is on', 'Turn off to pause this topic', enabled, x => (enabled = x)),
    err,
    h('div', { class: 'row' },
      t ? h('button', { class: 'danger', onclick: del }, 'Delete') : null,
      h('span', { class: 'sp' }),
      h('button', { onclick: () => m.close(undefined) }, 'Cancel'),
      h('button', { class: 'pri', onclick: save }, t ? 'Save' : 'Watch topic'))
  ].filter(Boolean) as Node[], { size: 'wide' })
  return m.done
}

/** API keys, the default citation style and the default databases. Resolves to the saved settings. */
export async function researchSettingsDialog(): Promise<ResearchSettings | undefined> {
  let s: ResearchSettings
  try { s = await researchApi.settings() } catch (e) { toast(errText(e)); return undefined }
  const err = h('div', { class: 'err', role: 'alert' })
  const oa = h('input', { type: 'text', value: s.keys.openalex, placeholder: 'Paste your key', spellcheck: false }) as HTMLInputElement
  const s2 = h('input', { type: 'text', value: s.keys.semanticScholar, placeholder: 'Optional', spellcheck: false }) as HTMLInputElement
  const email = h('input', { type: 'text', value: s.keys.email, placeholder: 'name@example.com', spellcheck: false }) as HTMLInputElement
  const style = select<CiteStyle>((Object.keys(STYLE_NAME) as CiteStyle[]).map(x => [x, STYLE_NAME[x]]), s.style)
  const sources = sourcePicker(s.sources)
  const link = (url: string, label: string) => h('button', { class: 'link', onclick: () => void newsApi.openLink(url).catch(e => toast(errText(e))) }, label)
  const save = async () => {
    const next: ResearchSettings = {
      keys: { openalex: oa.value.trim(), semanticScholar: s2.value.trim(), email: email.value.trim() },
      style: style.value as CiteStyle, sources: sources.read()
    }
    if (!next.sources.length) { err.textContent = 'Choose at least one database.'; return }
    if (next.keys.email && !/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(next.keys.email)) { err.textContent = 'That email address doesn’t look right.'; email.focus(); return }
    try { await researchApi.saveSettings(next); toast('Research settings saved'); m.close(next) } catch (e) { err.textContent = errText(e) }
  }
  const m = modal<ResearchSettings>('Research settings', [
    field('OpenAlex key', oa),
    h('div', { class: 'hint' }, 'OpenAlex needs a free key for regular use; without one only a small shared daily allowance is left. ', link('https://openalex.org/settings/api', 'Get a free key at openalex.org')),
    field('Semantic Scholar key', s2),
    h('div', { class: 'hint' }, 'Without a key Semantic Scholar is often busy. ', link('https://www.semanticscholar.org/product/api#api-key-form', 'Request a key')),
    field('Email for Unpaywall', email),
    h('div', { class: 'hint' }, 'Unpaywall finds free PDFs of paywalled papers. It asks for an email address and sends nothing to it.'),
    field('Citation style', style),
    h('div', { class: 'lab', style: 'margin-top:4px' }, 'Databases for new searches'),
    sources.el,
    err,
    h('div', { class: 'row' }, h('span', { class: 'sp' }), h('button', { onclick: () => m.close(undefined) }, 'Cancel'), h('button', { class: 'pri', onclick: save }, 'Save'))
  ], { size: 'wide' })
  return m.done
}

/** Journals and websites left out of every search and topic. */
export async function researchBlockedDialog(): Promise<void> {
  let items: string[] = []
  try { items = await researchApi.blocked() } catch (e) { toast(errText(e)) }
  const err = h('div', { class: 'err', role: 'alert' })
  const list = h('div', { class: 'feedlist' })
  const draw = () => list.replaceChildren(...(items.length ? items.map(d => h('div', {}, h('span', { title: d }, d),
    h('button', { class: 'ic', 'aria-label': `Unblock ${d}`, onclick: async () => {
      try { await researchApi.unblock(d); items = items.filter(x => x !== d); draw() } catch (e) { err.textContent = errText(e) }
    } }, icon('x')))) : [h('div', { class: 'hint', style: 'border:0;background:none' }, 'Nothing blocked.')]))
  draw()
  const input = h('input', { type: 'text', placeholder: 'Journal name or website, e.g. example.ru', 'aria-label': 'Journal or website to block' }) as HTMLInputElement
  const add = async () => {
    if (!input.value.trim()) return
    try {
      const d = await researchApi.block(input.value.trim())
      if (!items.includes(d)) items.push(d)
      items.sort((a, b) => a.localeCompare(b))
      input.value = ''
      err.textContent = ''
      draw()
    } catch (e) { err.textContent = errText(e) }
  }
  input.addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); void add() } })
  const m = modal<void>('Blocked journals and websites', [
    h('div', { class: 'hint' }, 'Papers from these journals or websites are left out of every search and topic. Papers in your library stay.'),
    list,
    h('div', { class: 'inl' }, input, h('button', { onclick: add }, icon('ban'), 'Block')),
    err,
    h('div', { class: 'row' }, h('button', { class: 'pri', onclick: () => m.close() }, 'Done'))
  ])
  await m.done
}
