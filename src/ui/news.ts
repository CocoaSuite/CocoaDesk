import { listen } from '@tauri-apps/api/event'
import { newsApi, errText, type NewsView, type NewsFilter, type Story, type NewsItem, type TopicSummary, type Reading, type Digest } from '../api'
import { h, fmtMinutes } from '../util'
import { icon } from './icons'
import { toast, menuAt } from './dialogs'
import { topicDialog, blockedDialog } from './topic'

const timeFmt = new Intl.DateTimeFormat('en-GB', { hour: '2-digit', minute: '2-digit' })
const dayFmt = new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short' })

function ago(iso: string): string {
  const t = new Date(iso).getTime()
  const mins = Math.max(0, Math.round((Date.now() - t) / 60000))
  if (mins < 1) return 'just now'
  if (mins < 60 * 24) return `${fmtMinutes(mins)} ago`
  const days = Math.round(mins / 1440)
  return days < 7 ? (days === 1 ? 'yesterday' : `${days} days ago`) : dayFmt.format(new Date(t))
}
const localTime = (naive: string) => timeFmt.format(new Date(naive))
const minutesSince = (naive: string) => Math.round((Date.now() - new Date(naive).getTime()) / 60000)

function everyText(m: number): string {
  if (m % 1440 === 0) return m === 1440 ? 'Daily' : `Every ${m / 1440} days`
  if (m % 60 === 0) return m === 60 ? 'Every hour' : `Every ${m / 60} hours`
  return `Every ${m} min`
}

function initials(it: NewsItem): string {
  const s = (it.sourceName || it.domain || '?').replace(/^www\./, '')
  const words = s.split(/[\s.]+/).filter(Boolean)
  return (words.length > 1 && it.sourceName ? words[0][0] + words[1][0] : s.slice(0, 2)).toUpperCase()
}

export class NewsScreen {
  el = h('main', { 'aria-label': 'News' })
  private topicId: number | null = null
  private filter: NewsFilter = 'all'
  private view: NewsView | null = null
  private checking = new Set<number>()
  private expanded = new Set<string>()
  private loading: Promise<void> | null = null
  /** The story open in the reader, and its pane (kept across redraws so scrolling stays put). */
  private reading: { itemId: number; el: HTMLElement } | null = null

  constructor() {
    document.addEventListener('keydown', e => {
      if (e.key === 'Escape' && this.reading && this.el.isConnected && !document.querySelector('.modal,.ctx')) this.closeReader()
    })
    void listen<number>('news:checking', e => { this.checking.add(e.payload); if (this.view) this.render() })
    void listen<number>('news:changed', e => { this.checking.delete(e.payload); void this.load() })
  }

  async load(): Promise<void> {
    if (this.loading) return this.loading
    this.loading = (async () => {
      try {
        this.view = await newsApi.view(this.topicId, this.filter)
        this.topicId = this.view.topic?.id ?? null
        this.render()
      } catch (e) {
        this.el.replaceChildren(h('div', { class: 'fatal' }, 'Could not load the news: ' + errText(e)))
      } finally { this.loading = null }
    })()
    return this.loading
  }

  private render() {
    const v = this.view!
    const focusKey = (document.activeElement as HTMLElement | null)?.dataset?.key
    const blockedBtn = h('button', { onclick: () => this.manageBlocked() }, icon('ban'), v.blockedSites.length ? `Blocked websites (${v.blockedSites.length})` : 'Blocked websites')
    const head = h('div', { class: 'head' },
      h('div', { class: 't' }, h('h1', {}, 'News'),
        h('div', { class: 'sub' }, v.topics.length ? `${v.topics.length} ${v.topics.length === 1 ? 'topic' : 'topics'}, checked on their own schedule` : 'Follow topics you care about')),
      blockedBtn,
      h('button', { class: 'pri', onclick: () => this.editTopic(null) }, icon('plus'), 'New topic'))
    if (!v.topic) {
      this.el.replaceChildren(head, h('div', { class: 'empty' },
        h('h2', {}, 'Follow a topic'),
        h('div', {}, 'Choose some words, and CocoaDesk checks news websites and the feeds you add on a schedule you set. Only stories that mention your words are kept — leave out websites and words you don’t want to see.'),
        h('div', { class: 'row' }, h('button', { class: 'pri', onclick: () => this.editTopic(null) }, icon('plus'), 'New topic'))))
      return
    }
    const cols = h('div', { class: 'ncols' + (this.reading ? ' reading' : '') }, this.topicList(v), this.topicPane(v))
    if (this.reading) cols.append(this.reading.el)
    this.el.replaceChildren(head, cols)
    if (focusKey) (this.el.querySelector(`[data-key="${CSS.escape(focusKey)}"]`) as HTMLElement | null)?.focus()
  }

  private topicList(v: NewsView): HTMLElement {
    return h('nav', { class: 'card topics', 'aria-label': 'Topics' }, ...v.topics.map(t => {
      const sub = !t.enabled ? 'Paused' : this.checking.has(t.id) ? 'Checking…' : t.lastError ? 'Problem with a source' : everyText(t.intervalMinutes)
      return h('button', {
        class: t.id === v.topic?.id ? 'on' : '', 'data-key': `t${t.id}`, 'aria-current': t.id === v.topic?.id ? 'true' : null,
        onclick: () => { this.topicId = t.id; this.expanded.clear(); this.reading = null; void this.load() }
      }, h('span', { class: 'tn' }, h('b', {}, t.name), h('span', { class: t.lastError && t.enabled ? 'bad' : '' }, sub)),
      t.unread ? h('span', { class: 'badge', 'aria-label': `${t.unread} unread` }, String(t.unread)) : null)
    }))
  }

  private statusLine(t: TopicSummary): HTMLElement {
    const busy = this.checking.has(t.id)
    const parts: string[] = []
    if (busy) parts.push('Checking now…')
    else if (!t.enabled) parts.push('Paused')
    else if (!t.lastChecked) parts.push('Not checked yet')
    else {
      const m = minutesSince(t.lastChecked)
      parts.push(m < 1 ? 'Checked just now' : `Checked ${fmtMinutes(m)} ago`)
      if (t.nextCheck) parts.push(`next at ${localTime(t.nextCheck)}`)
    }
    const nSrc = t.feeds.length + (t.search ? 1 : 0)
    parts.push(`${nSrc} ${nSrc === 1 ? 'source' : 'sources'}`)
    if (t.activeFrom && t.activeTo) parts.push(`${t.activeFrom}–${t.activeTo} only`)
    return h('div', { class: 'nstat' + (busy ? ' busy' : t.enabled ? '' : ' off') }, h('i'), parts.join(' · '))
  }

  private topicPane(v: NewsView): HTMLElement {
    const t = v.topic!
    const seg = h('div', { class: 'seg', role: 'group', 'aria-label': 'Show' },
      ...(['all', 'unread', 'saved'] as NewsFilter[]).map(f => h('button', {
        class: this.filter === f ? 'on' : '', 'aria-pressed': this.filter === f ? 'true' : 'false',
        onclick: () => { this.filter = f; void this.load() }
      }, f === 'all' ? 'All' : f === 'unread' ? 'Unread' : 'Saved')))
    const pane = h('section', { style: 'display:flex;flex-direction:column;gap:12px;min-width:0', 'aria-label': t.name },
      h('div', { class: 'head' },
        h('div', { class: 't' }, h('h1', { style: 'font-size:17px' }, t.name), this.statusLine(t)),
        h('button', { disabled: this.checking.has(t.id) || !t.enabled, onclick: () => this.checkNow(t.id) }, icon('refresh'), 'Check now'),
        h('button', { disabled: !t.unread, onclick: async () => { await newsApi.markTopicRead(t.id); await this.load() } }, icon('check'), 'Mark all read'),
        h('button', { onclick: () => this.editTopic(t) }, icon('sliders'), 'Edit topic')),
      t.lastError && t.enabled ? h('div', { class: 'nerr', role: 'status' }, t.lastError) : null,
      h('div', { class: 'bar' }, seg, h('span', { class: 'sp' }), h('span', { class: 'hint' }, 'Newest first')))
    if (!v.stories.length) {
      const msg = this.filter === 'saved' ? 'No saved stories. Use the bookmark on a story to keep it.'
        : this.filter === 'unread' ? 'You’re all caught up.'
          : !t.enabled ? 'This topic is paused. Turn checking on in Edit topic.'
            : this.checking.has(t.id) || !t.lastChecked ? 'Looking for stories…' : 'No stories match yet. They’ll appear here as they’re published.'
      pane.append(h('div', { class: 'card cardpad hint' }, msg))
    } else {
      if (v.digest && this.filter === 'all' && !this.reading) pane.append(this.digestCard(v.digest))
      pane.append(h('div', { class: 'card stories' }, ...v.stories.map(s => this.storyRow(s))))
    }
    return pane
  }

  private storyRow(s: Story): HTMLElement {
    const it = s.lead
    const when = it.publishedAt ?? it.fetchedAt
    const open = (item: NewsItem) => () => this.openReader(item.id)
    const saved = s.lead.saved || s.also.some(a => a.saved)
    const sources = [it, ...s.also]
    const tags = h('div', { class: 'tags' }, ...it.matched.slice(0, 4).map(m => h('span', { class: 'chip sel' }, m.replace(/^"|"$/g, ''))))
    if (s.also.length) {
      const isOpen = this.expanded.has(it.story)
      tags.append(h('button', {
        class: 'chip', style: 'cursor:pointer;background:none', 'aria-expanded': isOpen ? 'true' : 'false',
        onclick: () => { isOpen ? this.expanded.delete(it.story) : this.expanded.add(it.story); this.render() }
      }, `Also on ${s.also.length} other ${s.also.length === 1 ? 'website' : 'websites'}`))
    }
    const body = h('div', { class: 'bd' },
      h('button', { class: 'tt', 'data-key': `s${it.id}`, onclick: open(it) }, it.title),
      h('span', { class: 'meta' }, [it.sourceName || it.domain, ago(when)].filter(Boolean).join(' · ')),
      it.summary ? h('span', { class: 'sum' }, it.summary) : null,
      tags.childElementCount ? tags : null)
    if (s.also.length && this.expanded.has(it.story)) {
      body.append(h('div', { class: 'also' }, ...s.also.map(a => h('button', { onclick: open(a) }, h('span', {}, `${a.sourceName || a.domain}: `), a.title))))
    }
    const more = h('button', { class: 'ic', 'aria-label': 'More', onclick: () => {
      const domains = [...new Set(sources.map(x => x.domain).filter(Boolean))]
      menuAt(more, [
        { label: s.unread ? 'Mark as read' : 'Mark as unread', run: async () => { await newsApi.setStoryRead(it.id, s.unread); await this.load() } },
        { label: 'Open in browser', run: async () => { try { await newsApi.openStory(it.id); await this.load() } catch (e) { toast(errText(e)) } } },
        { label: 'Copy link', run: async () => { try { await navigator.clipboard.writeText(it.url); toast('Link copied') } catch { toast('Couldn’t copy the link') } } },
        'sep',
        ...domains.map(d => ({ label: `Block ${d} for all topics`, danger: true, run: () => this.block(d) }))
      ])
    } }, icon('more'))
    const current = this.reading && sources.some(x => x.id === this.reading!.itemId)
    const row = h('article', { class: 'st' + (s.unread ? ' unread' : '') + (current ? ' cur' : '') },
      h('span', { class: 'src', 'aria-hidden': 'true' }, initials(it)),
      body,
      h('div', { class: 'side-b' },
        s.unread ? h('span', { class: 'udot', role: 'img', 'aria-label': 'Unread' }) : null,
        h('button', { class: 'ic' + (saved ? ' saved' : ''), 'aria-label': saved ? 'Remove from saved' : 'Save for later', 'aria-pressed': saved ? 'true' : 'false',
          onclick: async () => { await newsApi.setSaved(it.id, !saved); for (const a of s.also) if (a.saved && saved) await newsApi.setSaved(a.id, false); await this.load() } }, icon('bookmark')),
        more))
    if (this.reading) row.addEventListener('click', e => { if (!(e.target as HTMLElement).closest('button')) this.openReader(it.id) })
    return row
  }

  // ---------- today ----------

  /** A paragraph about today assembled from counts and headlines; nothing is generated. */
  private digestCard(d: Digest): HTMLElement {
    // Headlines ending in their own full stop don't get a second one.
    const link = (id: number, title: string) => h('button', { class: 'ln', onclick: () => this.openReader(id) }, `«${title.replace(/[.。]\s*$/, '')}»`)
    const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`
    const p = h('p', {}, `So far today: ${plural(d.stories, 'new story', 'new stories')} from ${plural(d.websites, 'website', 'websites')}.`)
    const [first, ...rest] = d.top
    const parts: Node[] = [p]
    if (first) {
      p.append(' ', first.websites > 1 ? 'Most covered: ' : 'Latest: ', link(first.itemId, first.title), first.websites > 1 ? ` — on ${first.websites} websites.` : '.')
      if (first.lead) parts.push(h('div', { class: 'lead' }, first.lead))
    }
    if (rest.length) {
      const q = h('p', {}, 'Also: ')
      rest.forEach((s, i) => {
        if (i) q.append(i === rest.length - 1 ? ' and ' : ', ')
        q.append(link(s.itemId, s.title))
        if (s.websites > 1) q.append(` (${s.websites} websites)`)
      })
      q.append('.')
      parts.push(q)
      for (const s of rest) if (s.lead && s.websites > 1) parts.push(h('div', { class: 'lead' }, s.lead))
    }
    if (d.names.length) {
      const names = d.names.length > 1 ? `${d.names.slice(0, -1).join(', ')} and ${d.names[d.names.length - 1]}` : d.names[0]
      parts.push(h('p', { class: 'mute' }, `Often in the headlines: ${names}.`))
    }
    return h('section', { class: 'card digest', 'aria-label': 'Today so far' },
      h('div', { class: 'ch' }, h('span', { class: 'lab' }, 'Today so far'), h('span', { class: 'hint' }, 'Counted from today’s headlines')),
      ...parts)
  }

  // ---------- reader ----------

  private openReader(itemId: number) {
    const el = h('section', { class: 'card reader', 'aria-label': 'Article', tabindex: '-1' })
    this.reading = { itemId, el }
    this.fillReader(el, null, null, itemId)
    this.render()
    void this.loadArticle(itemId, false)
  }

  private closeReader() {
    this.reading = null
    this.render()
  }

  private async loadArticle(itemId: number, refresh: boolean) {
    try {
      const r = await newsApi.readStory(itemId, refresh)
      if (this.reading?.itemId !== itemId) return
      this.fillReader(this.reading.el, r, null, itemId)
    } catch (e) {
      if (this.reading?.itemId !== itemId) return
      this.fillReader(this.reading.el, null, errText(e), itemId)
    }
    void this.load()
  }

  private fillReader(el: HTMLElement, r: Reading | null, failure: string | null, itemId: number) {
    const item = r?.item ?? this.findItem(itemId)
    const a = r?.article ?? null
    const openBrowser = async () => { try { await newsApi.openStory(itemId) } catch (e) { toast(errText(e)) } }
    const saved = !!item?.saved
    const bar = h('div', { class: 'rbar' },
      h('button', { class: 'ic', 'aria-label': 'Close the article', onclick: () => this.closeReader() }, icon('x')),
      h('span', { class: 'sp' }),
      item ? h('button', { class: 'ic' + (saved ? ' saved' : ''), 'aria-label': saved ? 'Remove from saved' : 'Save for later', 'aria-pressed': saved ? 'true' : 'false',
        onclick: async () => { await newsApi.setSaved(itemId, !saved); await this.load(); if (r) this.fillReader(el, { ...r, item: { ...r.item, saved: !saved } }, failure, itemId) } }, icon('bookmark')) : null,
      h('button', { onclick: openBrowser }, 'Open in browser'))
    const body = h('div', { class: 'rbody' })
    const when = item ? ago(item.publishedAt ?? item.fetchedAt) : ''
    const meta = [a?.siteName || item?.sourceName || item?.domain, a?.byline, when].filter(Boolean).join(' · ')
    body.append(h('div', { class: 'rmeta' }, meta), h('h1', {}, item?.title || a?.title || ''))
    if (!r && !failure) {
      body.append(h('div', { class: 'rload', role: 'status' }, h('i'), `Getting the article from ${item?.domain || 'the website'}…`))
    } else if (!a) {
      body.append(h('div', { class: 'nerr', role: 'status' }, `Couldn’t show this article here. ${r?.error ?? failure ?? ''}`),
        h('div', { class: 'row', style: 'display:flex;gap:8px' },
          h('button', { onclick: () => { this.fillReader(el, null, null, itemId); void this.loadArticle(itemId, true) } }, icon('refresh'), 'Try again'),
          h('button', { class: 'pri', onclick: openBrowser }, 'Open in browser')))
    } else {
      if (a.image && !/<img\b/i.test(a.html)) body.append(h('img', { class: 'hero', src: a.image, alt: '', loading: 'lazy', onerror: (e: Event) => (e.target as HTMLElement).remove() }))
      const prose = h('div', { class: 'prose', lang: /[а-яё]/i.test(a.html.slice(0, 2000)) ? 'ru' : null })
      prose.innerHTML = a.html
      prose.addEventListener('click', e => {
        const link = (e.target as HTMLElement).closest('a')
        if (link?.href) { e.preventDefault(); void newsApi.openLink(link.href).catch(err => toast(errText(err))) }
      })
      for (const img of prose.querySelectorAll('img')) img.addEventListener('error', () => img.remove())
      body.append(prose)
    }
    if (r) {
      const foot = h('div', { class: 'rfoot' })
      if (a) foot.append(h('div', { class: 'hint' }, `Text from ${item?.domain || 'the website'}, shown without its ads and menus. Videos, comments and interactive parts are only in the original.`))
      if (r.also.length) {
        foot.append(h('div', { class: 'lab' }, 'Also covered by'),
          ...r.also.map(x => h('button', { class: 'alt', onclick: () => this.openReader(x.id) }, h('span', {}, `${x.sourceName || x.domain}: `), x.title)))
      }
      if (foot.childElementCount) body.append(foot)
    }
    el.replaceChildren(bar, body)
    el.scrollTop = 0
  }

  private findItem(id: number): NewsItem | null {
    for (const s of this.view?.stories ?? []) for (const x of [s.lead, ...s.also]) if (x.id === id) return x
    return null
  }

  private async block(domain: string) {
    try {
      await newsApi.blockSite(domain)
      toast(`${domain} is blocked for all topics`)
      await this.load()
    } catch (e) { toast(errText(e)) }
  }

  private async checkNow(id: number) {
    try {
      const out = await newsApi.checkNow(id)
      toast(out.error && !out.added ? 'The check ran into a problem' : out.added ? `${out.added} new ${out.added === 1 ? 'story' : 'stories'}` : 'No new stories')
    } catch (e) { toast(errText(e)) }
    await this.load()
  }

  private async editTopic(t: TopicSummary | null) {
    const res = await topicDialog(t, this.view?.blockedSites ?? [], () => this.manageBlocked())
    if (res === undefined) return
    this.topicId = res
    await this.load()
  }

  private async manageBlocked() {
    await blockedDialog(this.view?.blockedSites ?? [])
    await this.load()
  }
}
