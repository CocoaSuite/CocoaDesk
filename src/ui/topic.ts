import { newsApi, errText, type Topic, type TopicInput, type SearchEngine, type MatchIn, type NotifyMode } from '../api'
import { h } from '../util'
import { modal, field, select, confirmBox, toast, toggle } from './dialogs'
import { icon } from './icons'

const split = (s: string) => s.split(/[,\n;]/).map(x => x.trim()).filter(Boolean)
const join = (v: string[]) => v.join(', ')

const blank = (): TopicInput => ({
  name: '', enabled: true, include: [], exclude: [], blockedSites: [], feeds: [], search: 'google', language: 'ru',
  matchIn: 'titleAndSummary', intervalMinutes: 30, activeFrom: null, activeTo: null, notify: 'off', notifyWords: []
})

/** Interval as a number and a unit the person thinks in. */
function splitInterval(m: number): [number, number] {
  if (m % 1440 === 0) return [m / 1440, 1440]
  if (m % 60 === 0) return [m / 60, 60]
  return [m, 1]
}

/**
 * Creates or edits a news topic. Resolves to the topic id when saved, `null` when deleted,
 * `undefined` when cancelled.
 */
export async function topicDialog(t: Topic | null, globalBlocked: string[], manageBlocked: () => void): Promise<number | null | undefined> {
  const v: TopicInput = t ? { ...t } : blank()
  const err = h('div', { class: 'err', role: 'alert' })

  const name = h('input', { type: 'text', value: v.name, maxlength: 80, placeholder: 'e.g. Electric cars' }) as HTMLInputElement
  const include = h('input', { type: 'text', value: join(v.include), placeholder: 'электромобиль, Tesla, зарядная станция' }) as HTMLInputElement
  const exclude = h('input', { type: 'text', value: join(v.exclude), placeholder: 'реклама, вакансия' }) as HTMLInputElement
  const matchIn = select<MatchIn>([['titleAndSummary', 'Headline and summary'], ['title', 'Headline only']], v.matchIn)

  // Sources
  const useSearch = h('input', { type: 'checkbox', checked: v.search != null }) as HTMLInputElement
  const engine = select<SearchEngine>([['google', 'Google News'], ['bing', 'Bing News']], v.search ?? 'google', { 'aria-label': 'Search with' })
  const lang = select<string>([['ru', 'Russian edition'], ['en', 'English edition']], v.language, { 'aria-label': 'Edition' })
  const syncSearch = () => { engine.disabled = lang.disabled = !useSearch.checked }
  useSearch.addEventListener('change', syncSearch)
  syncSearch()

  let feeds = [...v.feeds]
  const feedList = h('div', { class: 'feedlist' })
  const drawFeeds = () => feedList.replaceChildren(...feeds.map((f, i) => h('div', {}, h('span', { title: f }, f),
    h('button', { class: 'ic', 'aria-label': `Remove ${f}`, onclick: () => { feeds.splice(i, 1); drawFeeds() } }, icon('x')))))
  drawFeeds()
  const feedIn = h('input', { type: 'text', placeholder: 'Website or feed address, e.g. habr.com' }) as HTMLInputElement
  const feedAdd = h('button', {}, 'Add feed') as HTMLButtonElement
  const addFeed = async () => {
    const a = feedIn.value.trim()
    if (!a) return
    feedAdd.disabled = true
    feedAdd.textContent = 'Looking…'
    try {
      const f = await newsApi.findFeed(a)
      if (!feeds.includes(f.url)) feeds.push(f.url)
      feedIn.value = ''
      err.textContent = ''
      drawFeeds()
      toast(`Added “${f.title}”`)
    } catch (e) { err.textContent = errText(e) }
    feedAdd.disabled = false
    feedAdd.textContent = 'Add feed'
  }
  feedAdd.addEventListener('click', addFeed)
  feedIn.addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); void addFeed() } })

  // Websites to leave out
  const blocked = h('input', { type: 'text', value: join(v.blockedSites), placeholder: 'e.g. example.ru, news.example.com' }) as HTMLInputElement
  const globalNote = h('div', { class: 'hint' },
    globalBlocked.length ? `Also left out for every topic: ${globalBlocked.slice(0, 6).join(', ')}${globalBlocked.length > 6 ? ` and ${globalBlocked.length - 6} more` : ''}. ` : 'You can also block websites for every topic. ',
    h('button', { class: 'link', onclick: () => { m.close(undefined); manageBlocked() } }, 'Manage blocked websites'))

  // Timing
  const [n, unit] = splitInterval(v.intervalMinutes)
  const every = h('input', { type: 'number', min: 1, max: 999, value: n, 'aria-label': 'Check every', style: 'width:80px;flex:none' }) as HTMLInputElement
  const everyUnit = select<number>([[1, 'minutes'], [60, 'hours'], [1440, 'days']], unit, { 'aria-label': 'Unit' })
  const from = h('input', { type: 'time', value: v.activeFrom ?? '', 'aria-label': 'From' }) as HTMLInputElement
  const to = h('input', { type: 'time', value: v.activeTo ?? '', 'aria-label': 'Until' }) as HTMLInputElement

  // Notifications
  const notify = select<NotifyMode>([['off', 'Don’t notify me'], ['all', 'Notify me about every new story'], ['words', 'Only when the headline mentions…']], v.notify)
  const notifyWords = h('input', { type: 'text', value: join(v.notifyWords), placeholder: 'e.g. release, запуск', 'aria-label': 'Notification words' }) as HTMLInputElement
  const syncNotify = () => { notifyWords.hidden = notify.value !== 'words' }
  notify.addEventListener('change', syncNotify)
  syncNotify()

  let enabled = v.enabled
  const save = async () => {
    const input: TopicInput = {
      name: name.value.trim(), enabled,
      include: split(include.value), exclude: split(exclude.value), blockedSites: split(blocked.value), feeds,
      search: useSearch.checked ? (engine.value as SearchEngine) : null, language: lang.value as 'ru' | 'en', matchIn: matchIn.value as MatchIn,
      intervalMinutes: Math.round(Math.max(1, Number(every.value) || 0) * Number(everyUnit.value)),
      activeFrom: from.value || null, activeTo: to.value || null,
      notify: notify.value as NotifyMode, notifyWords: split(notifyWords.value)
    }
    if (!input.name) { err.textContent = 'Give the topic a name.'; name.focus(); return }
    try {
      const id = await newsApi.saveTopic(t?.id ?? null, input)
      toast(t ? 'Topic saved — checking now' : 'Topic added — checking now')
      m.close(id)
    } catch (e) { err.textContent = errText(e) }
  }
  const del = async () => {
    if (!t) return
    if (!(await confirmBox('Delete topic?', `“${t.name}” and its stories will be removed. Saved stories go too.`))) return
    try { await newsApi.deleteTopic(t.id); toast('Topic deleted'); m.close(null) } catch (e) { err.textContent = errText(e) }
  }

  const m = modal<number | null>(t ? 'Edit topic' : 'New topic', [
    field('Name', name),
    h('div', { class: 'lab', style: 'margin-top:4px' }, 'What to look for'),
    field('Any of these words', include),
    h('div', { class: 'hint' }, 'Separate with commas. Word endings don’t matter — «электромобиль» also finds «электромобилей». Add * to match the start of a word (автомоб*), or put "quotes" around words that must match exactly.'),
    h('div', { class: 'g2' }, field('Leave out stories that mention', exclude), field('Look in', matchIn)),
    h('div', { class: 'lab', style: 'margin-top:4px' }, 'Where to look'),
    h('label', { class: 'sw', style: 'gap:8px' }, useSearch, 'Search news websites for these words'),
    h('div', { class: 'g2' }, engine, lang),
    h('div', { class: 'f', style: 'display:flex;flex-direction:column;gap:4px;font-size:12px;color:var(--mute)' }, 'Also read these feeds', feedList, h('div', { class: 'inl' }, feedIn, feedAdd)),
    field('Leave out these websites (this topic)', blocked),
    globalNote,
    h('div', { class: 'lab', style: 'margin-top:4px' }, 'When'),
    h('div', { class: 'g2' },
      h('div', { class: 'f', style: 'display:flex;flex-direction:column;gap:4px;font-size:12px;color:var(--mute)' }, 'Check every', h('div', { class: 'inl' }, every, everyUnit)),
      h('div', { class: 'f', style: 'display:flex;flex-direction:column;gap:4px;font-size:12px;color:var(--mute)' }, 'Only between (optional)', h('div', { class: 'inl' }, from, h('span', { class: 'mute' }, '–'), to))),
    field('Notifications', notify),
    notifyWords,
    toggle('Checking is on', 'Turn off to pause this topic', enabled, x => (enabled = x)),
    err,
    h('div', { class: 'row' },
      t ? h('button', { class: 'danger', onclick: del }, 'Delete') : null,
      h('span', { class: 'sp' }),
      h('button', { onclick: () => m.close(undefined) }, 'Cancel'),
      h('button', { class: 'pri', onclick: save }, t ? 'Save' : 'Add topic'))
  ].filter(Boolean) as Node[], { size: 'wide' })
  return m.done
}

/** The list of websites left out of every topic. */
export async function blockedDialog(initial: string[]): Promise<void> {
  let sites = [...initial]
  const err = h('div', { class: 'err', role: 'alert' })
  const list = h('div', { class: 'feedlist' })
  const draw = () => list.replaceChildren(...(sites.length ? sites.map(d => h('div', {}, h('span', {}, d),
    h('button', { class: 'ic', 'aria-label': `Unblock ${d}`, onclick: async () => {
      try { await newsApi.unblockSite(d); sites = sites.filter(x => x !== d); draw() } catch (e) { err.textContent = errText(e) }
    } }, icon('x')))) : [h('div', { class: 'hint', style: 'border:0;background:none' }, 'No websites blocked.')]))
  draw()
  const input = h('input', { type: 'text', placeholder: 'e.g. example.ru', 'aria-label': 'Website to block' }) as HTMLInputElement
  const add = async () => {
    if (!input.value.trim()) return
    try {
      const d = await newsApi.blockSite(input.value)
      if (!sites.includes(d)) sites.push(d)
      sites.sort()
      input.value = ''
      err.textContent = ''
      draw()
    } catch (e) { err.textContent = errText(e) }
  }
  input.addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); void add() } })
  const m = modal<void>('Blocked websites', [
    h('div', { class: 'hint' }, 'Stories from these websites are left out of every topic, and stories already found are removed (saved ones stay). Subdomains are included, so blocking example.ru also blocks news.example.ru.'),
    list,
    h('div', { class: 'inl' }, input, h('button', { onclick: add }, icon('ban'), 'Block')),
    err,
    h('div', { class: 'row' }, h('button', { class: 'pri', onclick: () => m.close() }, 'Done'))
  ])
  await m.done
}
