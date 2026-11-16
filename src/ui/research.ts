import {
  researchApi, newsApi, errText, paperPdfUrl, ALL_SOURCES, SOURCE_LABEL, SOURCE_HINT, KIND_NAME,
  type PaperView, type ResearchQuery, type SearchResult, type Overview, type ResearchTopicsView, type ResearchTopicSummary,
  type ResearchFilter, type LibraryView, type Shelf, type PaperReading, type CiteStyle, type ExploreKind, type ResearchDigest,
  type PaperSource, type PaperCollection
} from '../api'
import { h, fmtMinutes } from '../util'
import { icon } from './icons'
import { toast, menuAt, promptBox, confirmBox, type MenuItem } from './dialogs'
import { blankQuery, filterFields, researchTopicDialog, researchSettingsDialog, researchBlockedDialog, SORT_NAME, STYLE_NAME } from './researchdialogs'

type Tab = 'search' | 'watched' | 'library'
type ReaderTab = 'abstract' | 'text' | 'pdf'

const timeFmt = new Intl.DateTimeFormat('en-GB', { hour: '2-digit', minute: '2-digit' })
const localTime = (naive: string) => timeFmt.format(new Date(naive))
const minutesSince = (naive: string) => Math.round((Date.now() - new Date(naive).getTime()) / 60000)
const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`
const isRussian = (s: string) => /^[^a-zA-Z]*[а-яёА-ЯЁ]/.test(s)

function everyText(m: number): string {
  if (m % 1440 === 0) return m === 1440 ? 'Daily' : `Every ${m / 1440} days`
  const hrs = Math.round(m / 60)
  return hrs === 1 ? 'Every hour' : `Every ${hrs} hours`
}

const EXPLORE_TITLE: Record<ExploreKind, string> = { citedBy: 'Papers citing', references: 'References of', similar: 'Similar to' }

function authorsText(p: PaperView): string {
  const names = p.authors.map(a => a.family ? `${a.family} ${a.initials}`.trim() : a.name)
  if (names.length > 3) return `${names.slice(0, 3).join(', ')} ${isRussian(names[0]) ? 'и др.' : 'et al.'}`
  return names.join(', ')
}

function hostOf(url: string): string | null {
  try { return new URL(url).hostname.replace(/^www\./, '') } catch { return null }
}

async function copy(text: string, what: string) {
  try { await navigator.clipboard.writeText(text); toast(`${what} copied`) } catch { toast('Couldn’t copy') }
}

export class ResearchScreen {
  el = h('main', { 'aria-label': 'Research' })
  private tab: Tab = 'search'
  private style: CiteStyle = 'gost'
  private defaultSources: PaperSource[] = [...ALL_SOURCES]
  private settingsLoaded = false
  private checkingIds = new Set<number>()

  // search
  private query: ResearchQuery = blankQuery()
  private showFilters = false
  private searching = false
  private result: SearchResult | null = null
  private searchError: string | null = null
  private overview: Overview | null = null
  private explore: { from: PaperView; kind: ExploreKind; papers: PaperView[] | null; error: string | null } | null = null

  // watched
  private topicId: number | null = null
  private filter: ResearchFilter = 'all'
  private topics: ResearchTopicsView | null = null

  // library
  private shelf: Shelf = { kind: 'all' }
  private lib: LibraryView | null = null

  private reading: { paper: PaperView; el: HTMLElement; tab: ReaderTab; r: PaperReading | null; failure: string | null; pdf: 'idle' | 'loading' | 'ready' | string } | null = null
  private loading: Promise<void> | null = null

  constructor() {
    try { const t = localStorage.getItem('researchTab'); if (t === 'search' || t === 'watched' || t === 'library') this.tab = t } catch { /* storage unavailable */ }
    document.addEventListener('keydown', e => {
      if (e.key === 'Escape' && this.reading && this.el.isConnected && !document.querySelector('.modal,.ctx')) this.closeReader()
    })
  }

  /** Called when the background timer starts checking a topic. */
  checking(id: number) { this.checkingIds.add(id); if (this.tab === 'watched' && this.topics) this.render() }
  /** Called when a topic check finished. */
  changed(id: number) { this.checkingIds.delete(id); if (this.el.isConnected && this.tab !== 'search') void this.load() }

  async load(): Promise<void> {
    if (this.loading) return this.loading
    this.loading = (async () => {
      try {
        if (!this.settingsLoaded) {
          const s = await researchApi.settings()
          this.style = s.style
          this.defaultSources = s.sources.length ? s.sources : [...ALL_SOURCES]
          this.query.sources = [...this.defaultSources]
          this.settingsLoaded = true
        }
        if (this.tab === 'watched') {
          this.topics = await researchApi.topics(this.topicId, this.filter)
          this.topicId = this.topics.topic?.id ?? null
        }
        // The library view also carries collections and class subjects that the reader needs.
        if (this.tab === 'library' || !this.lib) this.lib = await researchApi.library(this.shelf)
        this.render()
      } catch (e) {
        this.el.replaceChildren(h('div', { class: 'fatal' }, 'Could not load Research: ' + errText(e)))
      } finally { this.loading = null }
    })()
    return this.loading
  }

  private setTab(t: Tab) {
    this.tab = t
    this.reading = null
    try { localStorage.setItem('researchTab', t) } catch { /* storage unavailable */ }
    this.render()
    void this.load()
  }

  // ---------- layout ----------

  private render() {
    const focusKey = (document.activeElement as HTMLElement | null)?.dataset?.key
    const tabs = h('div', { class: 'seg', role: 'tablist', 'aria-label': 'Research' },
      ...(['search', 'watched', 'library'] as Tab[]).map(t => h('button', {
        role: 'tab', class: this.tab === t ? 'on' : '', 'aria-selected': this.tab === t ? 'true' : 'false', onclick: () => this.setTab(t)
      }, t === 'search' ? 'Search' : t === 'watched' ? 'Watched' : 'Library')))
    const sub = this.tab === 'search' ? 'Look up papers in scholarly databases'
      : this.tab === 'watched' ? (this.topics?.topics.length ? `${plural(this.topics.topics.length, 'topic', 'topics')}, checked on their own schedule` : 'Get new papers on a topic as they come out')
        : this.lib ? `${plural(this.lib.total, 'saved paper', 'saved papers')}, ${this.lib.unread} unread` : ''
    const head = h('div', { class: 'head' },
      h('div', { class: 't' }, h('h1', {}, 'Research'), h('div', { class: 'sub' }, sub)),
      tabs,
      h('button', { onclick: () => this.manageBlocked() }, icon('ban'), 'Blocked'),
      h('button', { class: 'ic', 'aria-label': 'Research settings', title: 'Research settings', onclick: () => this.openSettings() }, icon('gear')),
      this.tab === 'watched' ? h('button', { class: 'pri', onclick: () => this.editTopic(null, null) }, icon('plus'), 'New topic') : null)
    const body = this.tab === 'search' ? this.searchTab() : this.tab === 'watched' ? this.watchedTab() : this.libraryTab()
    this.el.replaceChildren(head, body)
    if (focusKey) (this.el.querySelector(`[data-key="${CSS.escape(focusKey)}"]`) as HTMLElement | null)?.focus()
  }

  /** Wraps the main column(s) with the reader when a paper is open. */
  private withReader(cls: string, ...cols: HTMLElement[]): HTMLElement {
    const el = h('div', { class: cls + (this.reading ? ' reading' : '') }, ...cols)
    if (this.reading) el.append(this.reading.el)
    return el
  }

  // ---------- search ----------

  private searchTab(): HTMLElement {
    const q = this.query
    const input = h('input', { type: 'search', value: q.text, placeholder: 'машинное обучение в медицине', 'aria-label': 'Search for papers', 'data-key': 'q' }) as HTMLInputElement
    input.addEventListener('input', () => (q.text = input.value))
    input.addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); void this.runSearch() } })
    const filters = filterFields(q, { sort: true })
    const applyFilters = () => Object.assign(q, filters.read())
    const panel = this.showFilters ? h('div', { class: 'card cardpad rfilters' }, ...filters.els,
      h('div', { class: 'row' }, h('span', { class: 'sp' }), h('button', { onclick: () => { Object.assign(q, blankQuery(q.sources), { text: q.text }); this.render() } }, 'Clear filters'),
        h('button', { class: 'pri', onclick: () => { applyFilters(); void this.runSearch() } }, 'Apply'))) : null
    const nFilters = [q.fromYear != null || q.toYear != null, q.openAccess, q.minCitations != null, q.kind !== 'any', q.language != null, q.exclude.length > 0].filter(Boolean).length
    const bar = h('div', { class: 'rsearch' }, input,
      h('button', { class: this.showFilters ? 'on' : '', 'aria-expanded': this.showFilters ? 'true' : 'false', onclick: () => { if (this.showFilters) applyFilters(); this.showFilters = !this.showFilters; this.render() } },
        icon('filter'), nFilters ? `Filters (${nFilters})` : 'Filters'),
      h('button', { class: 'pri', disabled: this.searching, onclick: () => { if (this.showFilters) applyFilters(); void this.runSearch() } }, icon('search'), this.searching ? 'Searching…' : 'Search'))

    const r = this.result
    const errFor = (s: PaperSource) => r?.errors.find(e => e.startsWith(SOURCE_LABEL[s]))
    const chips = h('div', { class: 'dbchips', role: 'group', 'aria-label': 'Databases' }, ...ALL_SOURCES.map(s => {
      const on = q.sources.includes(s)
      const count = r?.counts.find(([x]) => x === s)?.[1]
      const err = on && r?.asked.includes(s) ? errFor(s) : undefined
      const label = SOURCE_LABEL[s] + (err ? ': no answer' : on && count != null ? ` ${count}` : '')
      return h('button', {
        class: 'chip' + (err ? ' bad' : on ? ' sel' : ''), 'aria-pressed': on ? 'true' : 'false', title: err ?? SOURCE_HINT[s],
        onclick: () => { if (this.showFilters) applyFilters(); q.sources = on ? q.sources.filter(x => x !== s) : ALL_SOURCES.filter(x => x === s || q.sources.includes(x)); this.render() }
      }, label)
    }), h('span', { class: 'sp' }),
    q.text.trim() ? h('button', { class: 'link', onclick: () => { applyFilters(); void this.editTopic(null, { ...q, text: q.text.trim() }) } }, 'Watch this search') : null)

    const main = h('section', { class: 'rmain', 'aria-label': 'Search' }, bar, panel, chips)
    if (this.explore) {
      const ex = this.explore
      main.replaceChildren(h('div', { class: 'head' },
        h('button', { onclick: () => { this.explore = null; this.reading = null; this.render() } }, icon('left'), 'Back'),
        h('div', { class: 't' }, h('h1', { style: 'font-size:16px' }, `${EXPLORE_TITLE[ex.kind]} «${ex.from.title.replace(/[.。]\s*$/, '')}»`))))
      if (ex.error) main.append(h('div', { class: 'nerr', role: 'status' }, ex.error))
      else if (!ex.papers) main.append(h('div', { class: 'rload', role: 'status' }, h('i'), 'Looking it up…'))
      else if (!ex.papers.length) main.append(h('div', { class: 'card cardpad hint' }, ex.kind === 'citedBy' ? 'No citing papers found yet.' : ex.kind === 'references' ? 'The databases don’t list this paper’s references.' : 'No similar papers found.'))
      else main.append(this.paperList(ex.papers, false))
      return this.withReader('rcols', main)
    }

    if (r?.errors.length) {
      const needsKey = r.errors.some(e => e.startsWith('OpenAlex') && /key|allowance/i.test(e))
      main.append(h('div', { class: 'nerr', role: 'status' }, ...r.errors.map(e => h('div', {}, e)),
        needsKey ? h('div', { style: 'margin-top:6px' }, h('button', { onclick: () => this.openSettings() }, icon('gear'), 'Add an OpenAlex key')) : null))
    }
    if (this.searchError) {
      const needsKey = /OpenAlex/.test(this.searchError) && /key|allowance/i.test(this.searchError)
      main.append(h('div', { class: 'nerr', role: 'status' }, this.searchError,
        needsKey ? h('div', { style: 'margin-top:6px' }, h('button', { onclick: () => this.openSettings() }, icon('gear'), 'Add an OpenAlex key')) : null))
    }
    if (this.overview && !this.reading) main.append(this.overviewCard(this.overview))
    if (this.searching && !r) main.append(h('div', { class: 'rload', role: 'status' }, h('i'), 'Asking the databases…'))
    else if (r) {
      if (!r.papers.length) main.append(h('div', { class: 'card cardpad hint' }, 'Nothing found. Try other words, fewer filters or more databases.'))
      else {
        main.append(h('div', { class: 'bar' }, h('span', { class: 'hint' }, plural(r.papers.length, 'paper', 'papers')), h('span', { class: 'sp' }),
          h('label', { class: 'hint', style: 'display:flex;align-items:center;gap:6px' }, 'Order',
            h('select', { style: 'width:auto', onchange: (e: Event) => { q.sort = (e.target as HTMLSelectElement).value as ResearchQuery['sort']; void this.runSearch() } },
              ...(Object.keys(SORT_NAME) as ResearchQuery['sort'][]).map(s => h('option', { value: s, selected: q.sort === s }, SORT_NAME[s]))))),
        this.paperList(r.papers, false))
      }
    } else if (!this.searchError) {
      main.append(h('div', { class: 'empty' }, h('h2', {}, 'Find papers'),
        h('div', {}, 'Type a topic in Russian or English. CocoaDesk asks the databases you pick at once, joins duplicates and shows a Wikipedia overview on top. Save papers to your library, read them here and copy references in GOST.')))
    }
    return this.withReader('rcols', main)
  }

  private async runSearch() {
    const q = this.query
    q.text = q.text.trim()
    if (!q.text) { (this.el.querySelector('input[type=search]') as HTMLInputElement | null)?.focus(); return }
    if (!q.sources.length) { toast('Choose at least one database'); return }
    this.searching = true
    this.searchError = null
    this.explore = null
    this.reading = null
    this.result = null
    this.overview = null
    this.render()
    const text = q.text
    void researchApi.overview(text).then(o => { if (this.query.text === text) { this.overview = o; if (this.tab === 'search' && !this.searching) this.render() } }).catch(() => { /* the overview is optional */ })
    try { this.result = await researchApi.search({ ...q, exclude: [...q.exclude], sources: [...q.sources] }) } catch (e) { this.searchError = errText(e) }
    this.searching = false
    if (this.tab === 'search') this.render()
  }

  private overviewCard(o: Overview): HTMLElement {
    return h('section', { class: 'card wiki', 'aria-label': 'Overview from Wikipedia', lang: o.lang },
      o.image ? h('img', { src: o.image, alt: '', loading: 'lazy', onerror: (e: Event) => (e.target as HTMLElement).remove() }) : null,
      h('div', { class: 'wb' },
        h('b', {}, o.title), o.description ? h('span', { class: 'hint' }, o.description) : null,
        h('p', {}, o.extract),
        h('button', { class: 'link', onclick: () => void newsApi.openLink(o.url).catch(e => toast(errText(e))) }, 'Read on Wikipedia')))
  }

  // ---------- paper rows ----------

  private paperList(papers: PaperView[], markUnread: boolean): HTMLElement {
    return h('div', { class: 'card stories' }, ...papers.map(p => this.paperRow(p, markUnread)))
  }

  private paperRow(p: PaperView, markUnread: boolean): HTMLElement {
    const unread = markUnread && !p.read
    const chips = h('div', { class: 'tags' },
      h('span', { class: 'chip' }, KIND_NAME[p.kind]),
      p.openAccess ? h('span', { class: 'chip ok' }, 'Free') : null,
      p.citations != null ? h('span', { class: 'chip' }, plural(p.citations, 'citation', 'citations')) : null,
      ...p.sources.map(s => h('span', { class: 'chip' }, SOURCE_LABEL[s])),
      p.subject ? h('span', { class: 'chip sel' }, p.subject) : null,
      p.taskCardId != null ? h('span', { class: 'chip warn' }, 'In Tasks') : null)
    const meta = [authorsText(p), p.year, p.venue].filter(Boolean).join(', ')
    const more = h('button', { class: 'ic', 'aria-label': 'More', onclick: () => menuAt(more, this.paperMenu(p)) }, icon('more'))
    const current = this.reading?.paper.id === p.id
    const row = h('article', { class: 'st paper' + (unread ? ' unread' : '') + (current ? ' cur' : ''), lang: isRussian(p.title) ? 'ru' : null },
      h('div', { class: 'bd' },
        h('button', { class: 'tt', 'data-key': `p${p.id}`, onclick: () => this.openReader(p) }, p.title),
        meta ? h('span', { class: 'meta' }, meta) : null,
        p.abstractText ? h('span', { class: 'sum' }, p.abstractText) : null,
        chips),
      h('div', { class: 'side-b' },
        unread ? h('span', { class: 'udot', role: 'img', 'aria-label': 'Unread' }) : null,
        h('button', { class: 'ic' + (p.saved ? ' saved' : ''), 'aria-label': p.saved ? 'Remove from library' : 'Save to library', 'aria-pressed': p.saved ? 'true' : 'false',
          onclick: () => this.update(p, { saved: !p.saved }, () => researchApi.setSaved(p.id, !p.saved), p.saved ? 'Removed from library' : 'Saved to library') }, icon('bookmark')),
        more))
    if (this.reading) row.addEventListener('click', e => { if (!(e.target as HTMLElement).closest('button')) this.openReader(p) })
    return row
  }

  private paperMenu(p: PaperView): MenuItem[] {
    const host = hostOf(p.url)
    return [
      { label: 'Open the paper’s page', run: () => this.openOut(p, 'page') },
      ...(p.pdfUrl ? [{ label: 'Open the PDF in browser', run: () => this.openOut(p, 'pdf') }] : []),
      'sep',
      { header: 'Copy reference' },
      ...(['gost', 'apa', 'bibtex'] as CiteStyle[]).map(s => ({ label: `Copy ${STYLE_NAME[s]}`, run: () => this.copyCite(p, s) })),
      'sep',
      { label: 'Papers citing this', run: () => this.runExplore(p, 'citedBy') },
      { label: 'Its references', run: () => this.runExplore(p, 'references') },
      { label: 'Similar papers', run: () => this.runExplore(p, 'similar') },
      'sep',
      { label: p.taskCardId != null ? 'Already in Tasks' : 'Add to Tasks…', run: () => { if (p.taskCardId == null) void this.addToTasks(p) } },
      { label: 'Link to a subject…', run: () => this.pickSubject(p) },
      { label: p.read ? 'Mark as unread' : 'Mark as read', run: () => this.update(p, { read: !p.read }, () => researchApi.setRead(p.id, !p.read)) },
      'sep',
      ...(p.venue ? [{ label: `Block the journal «${p.venue}»`, danger: true, run: () => this.block(p.venue) }] : []),
      ...(host ? [{ label: `Block ${host}`, danger: true, run: () => this.block(host) }] : [])
    ]
  }

  /** Changes a paper locally, runs the call, then refreshes the current tab. */
  private async update(p: PaperView, change: Partial<PaperView>, call: () => Promise<unknown>, done?: string) {
    try {
      await call()
      this.patch(p.id, change)
      if (done) toast(done)
      this.render()
      if (this.tab !== 'search') void this.load()
    } catch (e) { toast(errText(e)) }
  }

  /** Applies a change to every copy of a paper the screen holds. */
  private patch(id: number, change: Partial<PaperView>) {
    const lists = [this.result?.papers, this.explore?.papers, this.topics?.papers, this.lib?.papers]
    for (const l of lists) for (const x of l ?? []) if (x.id === id) Object.assign(x, change)
    if (this.reading?.paper.id === id) {
      Object.assign(this.reading.paper, change)
      if (this.reading.r) Object.assign(this.reading.r.paper, change)
      this.fillReader()
    }
  }

  private async openOut(p: PaperView, what: 'page' | 'pdf' | 'doi') {
    try { await researchApi.open(p.id, what) } catch (e) { toast(errText(e)) }
  }

  private async copyCite(p: PaperView, s: CiteStyle) {
    try { const c = await researchApi.cite(p.id); await copy(c[s], `${STYLE_NAME[s]} reference`) } catch (e) { toast(errText(e)) }
  }

  private async runExplore(p: PaperView, kind: ExploreKind) {
    this.tab = 'search'
    this.explore = { from: p, kind, papers: null, error: null }
    this.reading = null
    this.render()
    try {
      const papers = await researchApi.explore(p.id, kind)
      if (this.explore?.from.id === p.id && this.explore.kind === kind) this.explore.papers = papers
    } catch (e) {
      if (this.explore?.from.id === p.id && this.explore.kind === kind) this.explore.error = errText(e)
    }
    if (this.tab === 'search') this.render()
  }

  private async addToTasks(p: PaperView, anchor?: HTMLElement) {
    try {
      const boards = await researchApi.taskBoards()
      if (!boards.length) { toast('Make a task board first, in Tasks'); return }
      const add = async (boardId: number, name: string) => {
        try {
          const card = await researchApi.addToTasks(p.id, boardId)
          this.patch(p.id, { taskCardId: card })
          toast(`Added to ${name}`)
          this.render()
        } catch (e) { toast(errText(e)) }
      }
      if (boards.length === 1) return add(boards[0].id, boards[0].name)
      const at = anchor ?? { x: innerWidth / 2 - 100, y: innerHeight / 3 }
      menuAt(at, [{ header: 'Add a “Read” card to' }, ...boards.map(b => ({ label: b.name, run: () => void add(b.id, b.name) }))])
    } catch (e) { toast(errText(e)) }
  }

  private pickSubject(p: PaperView, anchor?: HTMLElement) {
    const subjects = this.lib?.classSubjects ?? []
    if (!subjects.length) { toast('Add classes first; their subjects show up here'); return }
    menuAt(anchor ?? { x: innerWidth / 2 - 100, y: innerHeight / 3 }, [
      { header: 'Subject' },
      ...subjects.map(s => ({ label: s === p.subject ? `${s} (current)` : s, run: () => this.update(p, { subject: s }, () => researchApi.setSubject(p.id, s)) })),
      ...(p.subject ? ['sep' as const, { label: 'No subject', run: () => this.update(p, { subject: '' }, () => researchApi.setSubject(p.id, '')) }] : [])
    ])
  }

  private async block(value: string) {
    try {
      const v = await researchApi.block(value)
      toast(`${v} is blocked`)
      const drop = (l?: PaperView[] | null) => l?.filter(x => x.saved || (x.venue !== value && hostOf(x.url) !== value)) ?? null
      if (this.result) this.result.papers = drop(this.result.papers)!
      if (this.explore?.papers) this.explore.papers = drop(this.explore.papers)
      this.render()
      if (this.tab !== 'search') void this.load()
    } catch (e) { toast(errText(e)) }
  }

  // ---------- reader ----------

  private openReader(p: PaperView) {
    if (this.reading?.paper.id === p.id) return
    const el = h('section', { class: 'card reader', 'aria-label': 'Paper', tabindex: '-1' })
    this.reading = { paper: p, el, tab: 'abstract', r: null, failure: null, pdf: 'idle' }
    this.fillReader()
    this.render()
    void this.loadReading(p.id, false)
  }

  private closeReader() { this.reading = null; this.render() }

  private async loadReading(id: number, refresh: boolean) {
    try {
      const r = await researchApi.read(id, refresh)
      if (this.reading?.paper.id !== id) return
      this.reading.r = r
      this.reading.failure = null
      Object.assign(this.reading.paper, r.paper)
      this.patch(id, { read: true })
    } catch (e) {
      if (this.reading?.paper.id !== id) return
      this.reading.failure = errText(e)
      this.fillReader()
    }
  }

  private async loadPdf() {
    const rd = this.reading
    if (!rd || rd.pdf === 'loading' || rd.pdf === 'ready') return
    rd.pdf = 'loading'
    this.fillReader()
    try { await researchApi.pdf(rd.paper.id); rd.pdf = 'ready' } catch (e) { rd.pdf = errText(e) }
    if (this.reading === rd) this.fillReader()
  }

  private fillReader() {
    const rd = this.reading
    if (!rd) return
    const p = rd.r?.paper ?? rd.paper
    const ru = isRussian(p.title)
    const bar = h('div', { class: 'rbar' },
      h('button', { class: 'ic', 'aria-label': 'Close the paper', onclick: () => this.closeReader() }, icon('x')),
      h('span', { class: 'sp' }),
      h('button', { class: 'ic' + (p.saved ? ' saved' : ''), 'aria-label': p.saved ? 'Remove from library' : 'Save to library', 'aria-pressed': p.saved ? 'true' : 'false',
        onclick: () => this.update(p, { saved: !p.saved }, () => researchApi.setSaved(p.id, !p.saved), p.saved ? 'Removed from library' : 'Saved to library') }, icon('bookmark')),
      p.pdfUrl ? h('button', { onclick: async () => { try { if (await researchApi.savePdf(p.id)) toast('PDF saved') } catch (e) { toast(errText(e)) } } }, icon('import'), 'Save PDF') : null,
      h('button', { onclick: () => this.openOut(p, p.doi ? 'doi' : 'page') }, 'Open page'))

    const tabs = h('div', { class: 'seg', role: 'tablist', 'aria-label': 'Show' }, ...([['abstract', 'Abstract'], ['text', 'Full text'], ['pdf', 'PDF']] as [ReaderTab, string][]).map(([t, l]) =>
      h('button', { role: 'tab', class: rd.tab === t ? 'on' : '', 'aria-selected': rd.tab === t ? 'true' : 'false', disabled: t === 'pdf' && !p.pdfUrl,
        title: t === 'pdf' && !p.pdfUrl ? 'No free PDF found' : null,
        onclick: () => { rd.tab = t; if (t === 'pdf') void this.loadPdf(); this.fillReader() } }, l)))

    const meta = [authorsText(p), p.year, [p.venue, p.volume && `vol. ${p.volume}`, p.issue && `no. ${p.issue}`, p.pages && `pp. ${p.pages}`].filter(Boolean).join(', ')].filter(Boolean).join(', ')
    const body = h('div', { class: 'rbody' + (rd.tab === 'pdf' ? ' wide' : ''), lang: ru ? 'ru' : null },
      h('div', { class: 'rmeta' }, meta),
      h('h1', {}, p.title),
      h('div', { class: 'tags' }, h('span', { class: 'chip' }, KIND_NAME[p.kind]), p.openAccess ? h('span', { class: 'chip ok' }, 'Free') : null,
        p.citations != null ? h('span', { class: 'chip' }, plural(p.citations, 'citation', 'citations')) : null, ...p.sources.map(s => h('span', { class: 'chip' }, SOURCE_LABEL[s]))),
      tabs)

    if (rd.tab === 'abstract') {
      body.append(p.abstractText ? h('div', { class: 'prose' }, ...p.abstractText.split(/\n{2,}/).map(t => h('p', {}, t))) : h('div', { class: 'hint' }, 'The databases have no abstract for this paper. Try Full text.'))
      if (p.tldr) body.append(h('div', { class: 'tldr' }, h('div', { class: 'lab' }, 'Short summary from Semantic Scholar'), h('div', {}, p.tldr),
        h('div', { class: 'hint' }, 'Written by Semantic Scholar’s model, not by the authors. Check it against the paper.')))
      if (p.keywords.length) body.append(h('div', { class: 'tags' }, ...p.keywords.map(k => h('span', { class: 'chip sel' }, k))))
    } else if (rd.tab === 'text') {
      const a = rd.r?.article
      if (!rd.r && !rd.failure) body.append(h('div', { class: 'rload', role: 'status' }, h('i'), `Getting the text from ${hostOf(p.htmlUrl ?? p.url) ?? 'the website'}…`))
      else if (!a) {
        body.append(h('div', { class: 'nerr', role: 'status' }, `Couldn’t show the full text here. ${rd.r?.textError ?? rd.failure ?? ''}`),
          h('div', { style: 'display:flex;gap:8px' },
            h('button', { onclick: () => { rd.r = null; rd.failure = null; this.fillReader(); void this.loadReading(p.id, true) } }, icon('refresh'), 'Try again'),
            p.pdfUrl ? h('button', { onclick: () => { rd.tab = 'pdf'; void this.loadPdf(); this.fillReader() } }, 'Show the PDF') : null,
            h('button', { class: 'pri', onclick: () => this.openOut(p, 'page') }, 'Open page')))
      } else {
        const prose = h('div', { class: 'prose' })
        prose.innerHTML = a.html
        prose.addEventListener('click', e => {
          const link = (e.target as HTMLElement).closest('a')
          if (link?.href) { e.preventDefault(); void newsApi.openLink(link.href).catch(err => toast(errText(err))) }
        })
        for (const img of prose.querySelectorAll('img')) img.addEventListener('error', () => img.remove())
        body.append(prose, h('div', { class: 'hint' }, `Text from ${hostOf(a.url) ?? 'the website'}, without its menus. Formulas, tables and figures may look better in the PDF.`))
      }
    } else {
      if (rd.pdf === 'ready') body.append(h('iframe', { class: 'pdf', src: paperPdfUrl(p.id), title: `PDF of ${p.title}` }))
      else if (rd.pdf === 'loading' || rd.pdf === 'idle') body.append(h('div', { class: 'rload', role: 'status' }, h('i'), 'Downloading the PDF…'))
      else body.append(h('div', { class: 'nerr', role: 'status' }, `Couldn’t get the PDF. ${rd.pdf}`),
        h('div', { style: 'display:flex;gap:8px' }, h('button', { onclick: () => { rd.pdf = 'idle'; void this.loadPdf() } }, icon('refresh'), 'Try again'),
          h('button', { onclick: () => this.openOut(p, 'pdf') }, 'Open in browser')))
    }

    if (rd.tab !== 'pdf') body.append(this.readerFoot(p))
    const scroll = rd.el.scrollTop
    rd.el.replaceChildren(bar, body)
    rd.el.scrollTop = scroll
    if (rd.el.isConnected) this.markCurrent()
  }

  /** Highlights the open paper's row without redrawing the list. */
  private markCurrent() {
    for (const row of this.el.querySelectorAll<HTMLElement>('.st.paper')) {
      const key = (row.querySelector('.tt') as HTMLElement | null)?.dataset.key
      row.classList.toggle('cur', key === `p${this.reading?.paper.id}`)
    }
  }

  private readerFoot(p: PaperView): HTMLElement {
    const foot = h('div', { class: 'rfoot' })
    // Cite box
    const cites = this.reading?.r?.citations
    const out = h('pre', { class: 'cite' }, cites ? cites[this.style] : 'Preparing the reference…')
    const styles = h('div', { class: 'seg', role: 'group', 'aria-label': 'Citation style' }, ...(['gost', 'apa', 'bibtex'] as CiteStyle[]).map(s =>
      h('button', { class: this.style === s ? 'on' : '', 'aria-pressed': this.style === s ? 'true' : 'false', onclick: () => { this.style = s; this.fillReader() } }, STYLE_NAME[s])))
    foot.append(h('div', { class: 'lab' }, 'Reference'),
      h('div', { class: 'bar' }, styles, h('span', { class: 'sp' }), h('button', { disabled: !cites, onclick: () => cites && copy(cites[this.style], `${STYLE_NAME[this.style]} reference`) }, icon('copy'), 'Copy')),
      out)

    // Notes
    const note = h('textarea', { placeholder: 'Your notes on this paper', 'aria-label': 'Notes', rows: 3 }) as HTMLTextAreaElement
    note.value = p.note
    note.addEventListener('blur', () => { if (note.value !== p.note) void this.update(p, { note: note.value }, () => researchApi.setNote(p.id, note.value)) })
    foot.append(h('div', { class: 'lab' }, 'Notes'), note)

    // Collections and subject
    const cols = this.lib?.collections ?? []
    const colChips = h('div', { class: 'tags' }, ...cols.map(c => {
      const on = p.collections.includes(c.id)
      const next = on ? p.collections.filter(x => x !== c.id) : [...p.collections, c.id]
      return h('button', { class: 'chip' + (on ? ' sel' : ''), 'aria-pressed': on ? 'true' : 'false',
        onclick: () => this.update(p, { collections: next, saved: true }, () => researchApi.setCollections(p.id, next)) }, c.name)
    }), h('button', { class: 'chip', onclick: async () => {
      const name = await promptBox('New collection', 'Name', '', 'Create')
      if (!name) return
      try {
        const id = await researchApi.saveCollection(null, name)
        this.lib?.collections.push({ id, name, papers: 0 })
        const next = [...p.collections, id]
        await this.update(p, { collections: next, saved: true }, () => researchApi.setCollections(p.id, next))
      } catch (e) { toast(errText(e)) }
    } }, '+ New collection'))
    const subjects = this.lib?.classSubjects ?? []
    const subj = h('select', { 'aria-label': 'Subject', style: 'width:auto;max-width:100%', onchange: (e: Event) => {
      const v = (e.target as HTMLSelectElement).value
      void this.update(p, { subject: v }, () => researchApi.setSubject(p.id, v))
    } }, h('option', { value: '' }, 'No subject'), ...[...new Set([...subjects, ...(p.subject ? [p.subject] : [])])].map(s => h('option', { value: s, selected: s === p.subject }, s)))
    foot.append(h('div', { class: 'lab' }, 'Collections'), colChips,
      h('div', { class: 'bar' }, h('span', { class: 'hint' }, 'Subject'), subj, h('span', { class: 'sp' }),
        p.taskCardId != null ? h('span', { class: 'chip warn' }, 'In Tasks')
          : h('button', { onclick: (e: Event) => this.addToTasks(p, e.currentTarget as HTMLElement) }, icon('tasks'), 'Add to Tasks')))

    // Explore
    foot.append(h('div', { class: 'lab' }, 'Explore'), h('div', { class: 'bar' },
      h('button', { onclick: () => this.runExplore(p, 'citedBy') }, 'Cited by'),
      h('button', { onclick: () => this.runExplore(p, 'references') }, 'References'),
      h('button', { onclick: () => this.runExplore(p, 'similar') }, 'Similar')))
    return foot
  }

  // ---------- watched ----------

  private watchedTab(): HTMLElement {
    const v = this.topics
    if (!v) return h('div', { class: 'rload' }, h('i'), 'Loading…')
    if (!v.topic) {
      return h('div', { class: 'empty' }, h('h2', {}, 'Watch a topic'),
        h('div', {}, 'Choose some words, and CocoaDesk asks the databases on a schedule you set and collects new papers. You can also watch any search from the Search tab.'),
        h('div', { class: 'row' }, h('button', { class: 'pri', onclick: () => this.editTopic(null, null) }, icon('plus'), 'New topic')))
    }
    const list = h('nav', { class: 'card topics', 'aria-label': 'Topics' }, ...v.topics.map(t => {
      const sub = !t.enabled ? 'Paused' : this.checkingIds.has(t.id) ? 'Checking…' : t.lastError ? 'Problem with a database' : everyText(t.intervalMinutes)
      return h('button', {
        class: t.id === v.topic?.id ? 'on' : '', 'data-key': `t${t.id}`, 'aria-current': t.id === v.topic?.id ? 'true' : null,
        onclick: () => { this.topicId = t.id; this.reading = null; void this.load() }
      }, h('span', { class: 'tn' }, h('b', {}, t.name), h('span', { class: t.lastError && t.enabled ? 'bad' : '' }, sub)),
      t.unread ? h('span', { class: 'badge', 'aria-label': `${t.unread} new` }, String(t.unread)) : null)
    }))
    const t = v.topic
    const seg = h('div', { class: 'seg', role: 'group', 'aria-label': 'Show' }, ...(['all', 'unread', 'saved'] as ResearchFilter[]).map(f => h('button', {
      class: this.filter === f ? 'on' : '', 'aria-pressed': this.filter === f ? 'true' : 'false', onclick: () => { this.filter = f; void this.load() }
    }, f === 'all' ? 'All' : f === 'unread' ? 'New' : 'Saved')))
    const pane = h('section', { class: 'rmain', 'aria-label': t.name },
      h('div', { class: 'head' },
        h('div', { class: 't' }, h('h1', { style: 'font-size:17px' }, t.name), this.statusLine(t)),
        h('button', { disabled: this.checkingIds.has(t.id) || !t.enabled, onclick: () => this.checkNow(t.id) }, icon('refresh'), 'Check now'),
        h('button', { disabled: !t.unread, onclick: async () => { try { await researchApi.markTopicRead(t.id) } catch (e) { toast(errText(e)) } await this.load() } }, icon('check'), 'Mark all read'),
        h('button', { onclick: () => this.editTopic(t, null) }, icon('sliders'), 'Edit')),
      t.lastError && t.enabled ? h('div', { class: 'nerr', role: 'status' }, t.lastError) : null,
      h('div', { class: 'bar' }, seg, h('span', { class: 'sp' }), h('span', { class: 'hint' }, 'Newest first')))
    if (!v.papers.length) {
      const msg = this.filter === 'saved' ? 'No saved papers from this topic. Use the bookmark on a paper to keep it.'
        : this.filter === 'unread' ? 'No new papers.'
          : !t.enabled ? 'This topic is paused. Turn checking on in Edit.'
            : this.checkingIds.has(t.id) || !t.lastChecked ? 'Looking for papers…' : 'No papers match yet. They’ll appear here as they come out.'
      pane.append(h('div', { class: 'card cardpad hint' }, msg))
    } else {
      if (v.digest && this.filter === 'all' && !this.reading) pane.append(this.digestCard(v.digest, v.papers))
      pane.append(this.paperList(v.papers, true))
    }
    return this.withReader('ncols', list, pane)
  }

  private statusLine(t: ResearchTopicSummary): HTMLElement {
    const busy = this.checkingIds.has(t.id)
    const parts: string[] = []
    if (busy) parts.push('Checking now…')
    else if (!t.enabled) parts.push('Paused')
    else if (!t.lastChecked) parts.push('Not checked yet')
    else {
      const m = minutesSince(t.lastChecked)
      parts.push(m < 1 ? 'Checked just now' : `Checked ${fmtMinutes(m)} ago`)
      if (t.nextCheck) parts.push(`next at ${localTime(t.nextCheck)}`)
    }
    parts.push(plural(t.sources.length, 'database', 'databases'))
    return h('div', { class: 'nstat' + (busy ? ' busy' : t.enabled ? '' : ' off') }, h('i'), parts.join(', '))
  }

  /** "This week": a paragraph put together from counts; nothing is generated. */
  private digestCard(d: ResearchDigest, papers: PaperView[]): HTMLElement {
    const byId = (id: number) => papers.find(p => p.id === id)
    const link = (id: number, title: string) => h('button', { class: 'ln', onclick: () => { const p = byId(id); if (p) this.openReader(p) } }, `«${title.replace(/[.。]\s*$/, '')}»`)
    const parts: Node[] = [h('p', {}, `${plural(d.papers, 'new paper', 'new papers')} in ${plural(d.journals, 'journal', 'journals')}, ${d.openAccess} free to read.`)]
    if (d.mostCited && d.mostCited.citations) parts.push(h('p', {}, 'Most cited: ', link(d.mostCited.id, d.mostCited.title), ` (${plural(d.mostCited.citations, 'citation', 'citations')}${d.mostCited.venue ? `, ${d.mostCited.venue}` : ''}).`))
    if (d.newest && d.newest.id !== d.mostCited?.id) parts.push(h('p', {}, 'Newest: ', link(d.newest.id, d.newest.title), d.newest.venue ? `, ${d.newest.venue}.` : '.'))
    if (d.busiest && d.busiest[1] > 1) parts.push(h('p', { class: 'mute' }, `Busiest journal: ${d.busiest[0]} (${d.busiest[1]} papers).`))
    if (d.keywords.length) parts.push(h('p', { class: 'mute' }, `Frequent keywords: ${d.keywords.join(', ')}.`))
    return h('section', { class: 'card digest', 'aria-label': 'This week' },
      h('div', { class: 'ch' }, h('span', { class: 'lab' }, 'This week'), h('span', { class: 'hint' }, 'Counted from the papers found')), ...parts)
  }

  private async checkNow(id: number) {
    this.checkingIds.add(id)
    this.render()
    try {
      const out = await researchApi.checkNow(id)
      toast(out.error && !out.added ? 'The check ran into a problem' : out.added ? plural(out.added, 'new paper', 'new papers') : 'No new papers')
    } catch (e) { toast(errText(e)) }
    this.checkingIds.delete(id)
    await this.load()
  }

  private async editTopic(t: ResearchTopicSummary | null, start: ResearchQuery | null) {
    const res = await researchTopicDialog(t, start ?? { ...blankQuery(this.defaultSources), text: '' })
    if (res === undefined) return
    this.topicId = res
    this.filter = 'all'
    if (this.tab !== 'watched') this.setTab('watched')
    else await this.load()
  }

  // ---------- library ----------

  private libraryTab(): HTMLElement {
    const v = this.lib
    if (!v) return h('div', { class: 'rload' }, h('i'), 'Loading…')
    const same = (s: Shelf) => JSON.stringify(s) === JSON.stringify(this.shelf)
    const go = (s: Shelf) => () => { this.shelf = s; this.reading = null; void this.load() }
    const btn = (s: Shelf, name: string, count: number, key: string) => h('button', { class: same(s) ? 'on' : '', 'aria-current': same(s) ? 'true' : null, 'data-key': key, onclick: go(s) },
      h('span', { class: 'tn' }, h('b', {}, name)), count ? h('span', { class: 'badge' }, String(count)) : null)
    const nav = h('nav', { class: 'card topics', 'aria-label': 'Shelves' },
      btn({ kind: 'all' }, 'All papers', v.total, 'sh-all'),
      btn({ kind: 'unread' }, 'Unread', v.unread, 'sh-unread'),
      h('div', { class: 'lbl' }, 'Collections'),
      ...v.collections.map(c => btn({ kind: 'collection', id: c.id }, c.name, c.papers, `sh-c${c.id}`)),
      h('button', { class: 'add', onclick: () => this.newCollection() }, icon('plus'), 'New collection'),
      v.subjects.length ? h('div', { class: 'lbl' }, 'Subjects') : null,
      ...v.subjects.map(([s, n]) => btn({ kind: 'subject', name: s }, s, n, `sh-s${s}`)))

    const shelf = v.shelf
    const col = shelf.kind === 'collection' ? v.collections.find(c => c.id === shelf.id) : undefined
    const title = shelf.kind === 'all' ? 'All papers' : shelf.kind === 'unread' ? 'Unread' : shelf.kind === 'subject' ? shelf.name : col?.name ?? 'Collection'
    const pane = h('section', { class: 'rmain', 'aria-label': title },
      h('div', { class: 'head' },
        h('div', { class: 't' }, h('h1', { style: 'font-size:17px' }, title), h('div', { class: 'sub' }, plural(v.papers.length, 'paper', 'papers'))),
        col ? h('button', { onclick: () => this.renameCollection(col) }, icon('edit'), 'Rename') : null,
        col ? h('button', { class: 'danger', onclick: () => this.deleteCollection(col) }, icon('trash'), 'Delete') : null,
        h('button', { disabled: !v.papers.length, onclick: () => this.copyReferenceList(v.papers) }, icon('copy'), 'Copy reference list')))
    if (!v.papers.length) {
      pane.append(h('div', { class: 'card cardpad hint' }, shelf.kind === 'all' ? 'Your library is empty. Use the bookmark on any paper to save it here.'
        : shelf.kind === 'unread' ? 'You’ve read everything you saved.' : 'Nothing here yet. Add papers from the reader.'))
    } else pane.append(this.paperList(v.papers, true))
    return this.withReader('ncols', nav, pane)
  }

  private async newCollection() {
    const name = await promptBox('New collection', 'Name', '', 'Create')
    if (!name) return
    try { const id = await researchApi.saveCollection(null, name); this.shelf = { kind: 'collection', id }; await this.load() } catch (e) { toast(errText(e)) }
  }

  private async renameCollection(c: PaperCollection) {
    const name = await promptBox('Rename collection', 'Name', c.name)
    if (!name || name === c.name) return
    try { await researchApi.saveCollection(c.id, name); await this.load() } catch (e) { toast(errText(e)) }
  }

  private async deleteCollection(c: PaperCollection) {
    if (!(await confirmBox('Delete collection?', `“${c.name}” is removed. Its papers stay in your library.`))) return
    try { await researchApi.deleteCollection(c.id); this.shelf = { kind: 'all' }; await this.load() } catch (e) { toast(errText(e)) }
  }

  /** GOST list: Russian-language works first, then foreign ones, each alphabetical. */
  private async copyReferenceList(papers: PaperView[]) {
    try {
      const refs = await Promise.all(papers.map(p => researchApi.cite(p.id).then(c => c.gost)))
      const ru = refs.filter(isRussian).sort((a, b) => a.localeCompare(b, 'ru'))
      const other = refs.filter(r => !isRussian(r)).sort((a, b) => a.localeCompare(b, 'en'))
      await copy([...ru, ...other].map((r, i) => `${i + 1}. ${r}`).join('\n'), `Reference list (${refs.length})`)
    } catch (e) { toast(errText(e)) }
  }

  // ---------- dialogs ----------

  private async openSettings() {
    const s = await researchSettingsDialog()
    if (!s) return
    this.style = s.style
    this.defaultSources = s.sources
    if (!this.result) this.query.sources = [...s.sources]
    this.render()
  }

  private async manageBlocked() {
    await researchBlockedDialog()
    if (this.tab !== 'search') await this.load()
  }
}
