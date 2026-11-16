import { invoke, convertFileSrc } from '@tauri-apps/api/core'

export type WeekKind = 'top' | 'bottom'
export type Weeks = 'every' | 'top' | 'bottom'
export type ClassKind = 'lecture' | 'seminar' | 'lab' | 'practice' | 'sport' | 'other'
export type MarkStatus = 'done' | 'skipped' | 'cancelled'
export type Status = 'upcoming' | 'inProgress' | 'done' | 'skipped' | 'cancelled' | 'ended' | 'untracked'

export interface Period { number: number; start: string; end: string }
export interface Break { id: number; from: string; to: string; label: string }
export interface Semester {
  id: number; name: string; start: string; end: string; firstWeek: WeekKind
  periods: Period[]; breaks: Break[]; classCount: number
}
export interface ClassInput {
  subject: string; kind: ClassKind; teacher: string; room: string; note: string
  weekday: number; period: number; weeks: Weeks; validFrom: string | null; validTo: string | null
}
export interface Class extends ClassInput {
  id: number; semesterId: number; source: 'manual' | 'import'; importKey: string | null; createdAt: string
}
export interface Entry {
  class: Class; date: string; start: string; end: string; status: Status
  markedBy: 'auto' | 'user' | null; progress: number | null; minutesLeft: number | null; minutesUntil: number | null
}
export interface Day {
  date: string; weekday: number; isToday: boolean; inSemester: boolean; breakLabel: string | null
  weekKind: WeekKind | null; entries: Entry[]; otherWeek: Class[]
}
export interface Settings { autoMark: boolean; remindMinutes: number; closeToTray: boolean }
export interface WeekView {
  semesters: Semester[]; semester: Semester | null; monday: string; weekKind: WeekKind | null
  weekNumber: number | null; weekCount: number | null; days: Day[]; today: Day | null; now: string; settings: Settings
}
export interface SemesterInput { name: string; start: string; end: string; firstWeek: WeekKind }

export interface GroupColumn { column: number; label: string; detail: string }
export interface ImportItem {
  key: string; weekday: number; period: number; weeks: Weeks; subject: string; kind: ClassKind
  teacher: string; room: string; note: string; months: [number, number] | null; shared: boolean; raw: string
}
export interface ImportPreview {
  sheets: string[]; sheet: string; groups: GroupColumn[]; group: string; periods: Period[]; items: ImportItem[]; problems: string[]
}
export type ImportTarget = { kind: 'existing'; id: number } | { kind: 'new'; semester: SemesterInput }
export interface ImportRequest { sheet: string; group: string; target: ImportTarget; includedKeys: string[]; useFilePeriods: boolean }
export interface ImportDone { semesterId: number; added: number; updated: number; removed: number }
export interface PickedFile { token: string; name: string }

export const api = {
  weekView: (semesterId: number | null, date: string | null) => invoke<WeekView>('week_view', { semesterId, date }),
  saveSemester: (id: number | null, input: SemesterInput) => invoke<number>('save_semester', { id, input }),
  deleteSemester: (id: number) => invoke<void>('delete_semester', { id }),
  setPeriods: (semesterId: number, periods: Period[]) => invoke<void>('set_periods', { semesterId, periods }),
  addBreak: (semesterId: number, from: string, to: string, label: string) => invoke<number>('add_break', { semesterId, from, to, label }),
  deleteBreak: (id: number) => invoke<void>('delete_break', { id }),
  saveClass: (semesterId: number, id: number | null, input: ClassInput) => invoke<number>('save_class', { semesterId, id, input }),
  deleteClass: (id: number) => invoke<void>('delete_class', { id }),
  setStatus: (classId: number, date: string, status: MarkStatus | null) => invoke<void>('set_status', { classId, date, status }),
  saveSettings: (settings: Settings) => invoke<void>('save_settings', { settings }),
  pickTimetable: () => invoke<PickedFile | null>('pick_timetable'),
  importPreview: (token: string, sheet: string | null, group: string | null) => invoke<ImportPreview>('import_preview', { token, sheet, group }),
  importExclusions: (semesterId: number) => invoke<string[]>('import_exclusions', { semesterId }),
  importCommit: (token: string, request: ImportRequest) => invoke<ImportDone>('import_commit', { token, request })
}

/** Errors from the Rust side arrive as plain strings. */
export const errText = (e: unknown) => (typeof e === 'string' ? e : e instanceof Error ? e.message : 'Something went wrong.')

export const KIND_LABEL: Record<ClassKind, string> = {
  lecture: 'Lecture', seminar: 'Seminar', lab: 'Lab', practice: 'Practice', sport: 'Sport', other: 'Other'
}
export const WEEKS_LABEL: Record<Weeks, string> = { every: 'Every week', top: 'Top week only', bottom: 'Bottom week only' }

// ---------- news ----------

export type SearchEngine = 'google' | 'bing'
export type MatchIn = 'title' | 'titleAndSummary'
export type NotifyMode = 'off' | 'all' | 'words'
export type NewsFilter = 'all' | 'unread' | 'saved'

export interface TopicConfig {
  include: string[]; exclude: string[]; blockedSites: string[]; feeds: string[]
  search: SearchEngine | null; language: 'ru' | 'en'; matchIn: MatchIn; intervalMinutes: number
  activeFrom: string | null; activeTo: string | null; notify: NotifyMode; notifyWords: string[]
}
export interface TopicInput extends TopicConfig { name: string; enabled: boolean }
export interface Topic extends TopicInput {
  id: number; createdAt: string; lastChecked: string | null; lastError: string | null
}
export interface TopicSummary extends Topic { unread: number; nextCheck: string | null }
export interface NewsItem {
  id: number; topicId: number; url: string; title: string; summary: string; sourceName: string; domain: string
  publishedAt: string | null; fetchedAt: string; read: boolean; saved: boolean; matched: string[]; story: string
}
export interface Story { lead: NewsItem; also: NewsItem[]; unread: boolean }
export interface DigestStory { itemId: number; title: string; websites: number; lead: string | null }
export interface Digest { stories: number; websites: number; top: DigestStory[]; names: string[] }
export interface NewsView {
  topics: TopicSummary[]; topic: TopicSummary | null; stories: Story[]; filter: NewsFilter; blockedSites: string[]; now: string
  digest: Digest | null
}
export interface Article {
  url: string; title: string; byline: string | null; siteName: string | null; image: string | null; html: string; lead: string; words: number
}
export interface Reading { item: NewsItem; article: Article | null; error: string | null; also: NewsItem[] }
export interface CheckOutcome { topicId: number; topicName: string; added: number; error: string | null }

export const newsApi = {
  view: (topicId: number | null, filter: NewsFilter) => invoke<NewsView>('news_view', { topicId, filter }),
  saveTopic: (id: number | null, input: TopicInput) => invoke<number>('save_topic', { id, input }),
  deleteTopic: (id: number) => invoke<void>('delete_topic', { id }),
  checkNow: (id: number) => invoke<CheckOutcome>('check_topic_now', { id }),
  findFeed: (address: string) => invoke<{ url: string; title: string }>('find_feed', { address }),
  setStoryRead: (itemId: number, read: boolean) => invoke<void>('set_story_read', { itemId, read }),
  markTopicRead: (topicId: number) => invoke<void>('mark_topic_read', { topicId }),
  setSaved: (itemId: number, saved: boolean) => invoke<void>('set_saved', { itemId, saved }),
  blockSite: (site: string) => invoke<string>('block_site', { site }),
  unblockSite: (domain: string) => invoke<void>('unblock_site', { domain }),
  openStory: (itemId: number) => invoke<void>('open_story', { itemId }),
  readStory: (itemId: number, refresh = false) => invoke<Reading>('read_story', { itemId, refresh }),
  openLink: (url: string) => invoke<void>('open_link', { url })
}

// ---------- moodboards ----------

export interface BoardSummary { id: number; name: string; createdAt: string; updatedAt: string; items: number; cover: string | null }
export interface Board { id: number; name: string; scene: string }

export const boardApi = {
  list: () => invoke<BoardSummary[]>('boards_list'),
  load: (id: number) => invoke<Board>('board_load', { id }),
  create: (name: string) => invoke<number>('board_create', { name }),
  rename: (id: number, name: string) => invoke<void>('board_rename', { id, name }),
  duplicate: (id: number) => invoke<number>('board_duplicate', { id }),
  remove: (id: number) => invoke<void>('board_delete', { id }),
  save: (id: number, scene: string) => invoke<void>('board_save', { id, scene }),
  /** Stores image bytes and returns the image's id. */
  addImage: (bytes: Uint8Array) => invoke<string>('board_add_image', bytes, { headers: { 'Content-Type': 'application/octet-stream' } }),
  fetchImage: (url: string) => invoke<string>('board_fetch_image', { url }),
  overlay: (on: boolean) => invoke<void>('board_overlay', { on })
}

/** Where the window loads a stored board image from. */
export const boardImageUrl = (id: string) => convertFileSrc(id, 'boardimg')

// ---------- tasks ----------

export type Currency = 'usd' | 'rub'
export interface Label { id: string; name: string; color: string }
export interface BoardInput { name: string; color: string; commission: number; labels: Label[] }
export interface TaskBoard extends BoardInput { id: number; cards: number }
export interface TaskList { id: number; boardId: number; name: string; isDone: boolean; position: number }
export interface ChecklistItem { text: string; done: boolean }
export interface Salary { amount: number; currency: Currency; commission: number | null }
export interface CardInput {
  title: string; description: string; labels: string[]; due: string | null; checklist: ChecklistItem[]; salary: Salary | null
}
export interface Money {
  commission: number; grossUsd: number | null; grossRub: number | null; netUsd: number | null; netRub: number | null; rate: number | null
}
export interface Card extends CardInput {
  id: number; listId: number; position: number; paidAt: string | null; lockedRate: number | null
  doneAt: string | null; createdAt: string; money: Money | null
}
export interface ListView extends TaskList { cards: Card[] }
export interface RateInfo { value: number; date: string | null; manual: boolean }
export interface MonthEarnings { month: string; netUsd: number; netRub: number; cards: number; unpaidUsd: number }
export interface TasksView {
  boards: TaskBoard[]; board: TaskBoard | null; lists: ListView[]; rate: RateInfo | null; months: MonthEarnings[]
}

export const taskApi = {
  view: (boardId: number | null) => invoke<TasksView>('tasks_view', { boardId }),
  saveBoard: (id: number | null, input: BoardInput) => invoke<number>('task_board_save', { id, input }),
  deleteBoard: (id: number) => invoke<void>('task_board_delete', { id }),
  saveList: (boardId: number, id: number | null, name: string, isDone: boolean) => invoke<number>('task_list_save', { boardId, id, name, isDone }),
  deleteList: (id: number) => invoke<void>('task_list_delete', { id }),
  moveList: (id: number, index: number) => invoke<void>('task_list_move', { id, index }),
  saveCard: (listId: number, id: number | null, input: CardInput) => invoke<number>('task_card_save', { listId, id, input }),
  deleteCard: (id: number) => invoke<void>('task_card_delete', { id }),
  moveCard: (id: number, listId: number, index: number) => invoke<void>('task_card_move', { id, listId, index }),
  setPaid: (id: number, paid: boolean) => invoke<void>('task_card_paid', { id, paid }),
  refreshRate: () => invoke<RateInfo>('rate_refresh'),
  setManualRate: (value: number | null) => invoke<void>('rate_set_manual', { value })
}

// ---------- research ----------

export type PaperSource = 'openAlex' | 'arxiv' | 'semanticScholar' | 'europePmc' | 'cyberLeninka'
export type PaperKind = 'article' | 'preprint' | 'review' | 'conference' | 'book' | 'chapter' | 'thesis' | 'other'
export type KindFilter = 'any' | 'article' | 'preprint' | 'review'
export type SortBy = 'relevance' | 'newest' | 'mostCited'
export type CiteStyle = 'gost' | 'apa' | 'bibtex'
export type ExploreKind = 'citedBy' | 'references' | 'similar'
export type ResearchFilter = 'all' | 'unread' | 'saved'

export const SOURCE_LABEL: Record<PaperSource, string> = {
  openAlex: 'OpenAlex', arxiv: 'arXiv', semanticScholar: 'Semantic Scholar', europePmc: 'Europe PMC', cyberLeninka: 'CyberLeninka'
}
export const SOURCE_HINT: Record<PaperSource, string> = {
  openAlex: 'Every field, about 250 million works',
  arxiv: 'Preprints in physics, maths, computer science',
  semanticScholar: 'Every field, with citations',
  europePmc: 'Biology and medicine',
  cyberLeninka: 'Russian journals, free full text'
}
export const ALL_SOURCES: PaperSource[] = ['openAlex', 'arxiv', 'semanticScholar', 'europePmc', 'cyberLeninka']
export const KIND_NAME: Record<PaperKind, string> = {
  article: 'Article', preprint: 'Preprint', review: 'Review', conference: 'Conference paper', book: 'Book', chapter: 'Book chapter', thesis: 'Thesis', other: 'Other'
}

export interface Author { name: string; family: string; given: string; initials: string }
export interface Paper {
  key: string; title: string; authors: Author[]; year: number | null; date: string | null; venue: string; publisher: string
  volume: string; issue: string; pages: string; kind: PaperKind; language: string | null; abstractText: string; tldr: string | null
  citations: number | null; doi: string | null; arxiv: string | null; pmid: string | null; pmcid: string | null; openalex: string | null; s2: string | null
  url: string; pdfUrl: string | null; htmlUrl: string | null; openAccess: boolean; keywords: string[]; sources: PaperSource[]
}
export interface PaperView extends Paper {
  id: number; saved: boolean; read: boolean; isNew: boolean; note: string; subject: string; collections: number[]
  taskCardId: number | null; foundAt: string | null
}
export interface ResearchQuery {
  text: string; fromYear: number | null; toYear: number | null; openAccess: boolean; minCitations: number | null
  kind: KindFilter; language: 'ru' | 'en' | null; sources: PaperSource[]; exclude: string[]; blocked: string[]; sort: SortBy
}
export interface SearchResult { papers: PaperView[]; errors: string[]; asked: PaperSource[]; counts: [PaperSource, number][] }
export interface Overview { title: string; description: string; extract: string; image: string | null; url: string; lang: string }
export interface ResearchKeys { openalex: string; semanticScholar: string; email: string }
export interface ResearchSettings { keys: ResearchKeys; style: CiteStyle; sources: PaperSource[] }
export interface ResearchTopicConfig extends ResearchQuery { intervalMinutes: number; notify: boolean }
export interface ResearchTopicInput extends ResearchTopicConfig { name: string; enabled: boolean }
export interface ResearchTopic extends ResearchTopicInput { id: number; createdAt: string; lastChecked: string | null; lastError: string | null }
export interface ResearchTopicSummary extends ResearchTopic { unread: number; nextCheck: string | null }
export interface DigestPaper { id: number; title: string; citations: number | null; date: string | null; venue: string }
export interface ResearchDigest {
  papers: number; journals: number; openAccess: number; mostCited: DigestPaper | null; newest: DigestPaper | null
  busiest: [string, number] | null; keywords: string[]
}
export interface ResearchTopicsView {
  topics: ResearchTopicSummary[]; topic: ResearchTopicSummary | null; papers: PaperView[]; filter: ResearchFilter; digest: ResearchDigest | null
}
export type Shelf = { kind: 'all' } | { kind: 'unread' } | { kind: 'collection'; id: number } | { kind: 'subject'; name: string }
export interface PaperCollection { id: number; name: string; papers: number }
export interface LibraryView {
  shelf: Shelf; papers: PaperView[]; collections: PaperCollection[]; subjects: [string, number][]; classSubjects: string[]; total: number; unread: number
}
export interface Citations { gost: string; apa: string; bibtex: string }
export interface PaperReading { paper: PaperView; article: Article | null; textError: string | null; citations: Citations }
export interface ResearchCheck { topicId: number; topicName: string; added: number; error: string | null }

export const researchApi = {
  settings: () => invoke<ResearchSettings>('research_settings'),
  saveSettings: (settings: ResearchSettings) => invoke<void>('research_save_settings', { settings }),
  search: (query: ResearchQuery) => invoke<SearchResult>('research_search', { query }),
  overview: (text: string) => invoke<Overview | null>('research_overview', { text }),
  topics: (topicId: number | null, filter: ResearchFilter) => invoke<ResearchTopicsView>('research_topics_view', { topicId, filter }),
  saveTopic: (id: number | null, input: ResearchTopicInput) => invoke<number>('research_save_topic', { id, input }),
  deleteTopic: (id: number) => invoke<void>('research_delete_topic', { id }),
  checkNow: (id: number) => invoke<ResearchCheck>('research_check_now', { id }),
  markTopicRead: (topicId: number) => invoke<void>('research_mark_topic_read', { topicId }),
  library: (shelf: Shelf) => invoke<LibraryView>('research_library', { shelf }),
  setSaved: (paperId: number, saved: boolean) => invoke<void>('research_set_saved', { paperId, saved }),
  setRead: (paperId: number, read: boolean) => invoke<void>('research_set_read', { paperId, read }),
  setNote: (paperId: number, note: string) => invoke<void>('research_set_note', { paperId, note }),
  setSubject: (paperId: number, subject: string) => invoke<void>('research_set_subject', { paperId, subject }),
  setCollections: (paperId: number, collections: number[]) => invoke<void>('research_set_collections', { paperId, collections }),
  saveCollection: (id: number | null, name: string) => invoke<number>('research_save_collection', { id, name }),
  deleteCollection: (id: number) => invoke<void>('research_delete_collection', { id }),
  blocked: () => invoke<string[]>('research_blocked'),
  block: (value: string) => invoke<string>('research_block', { value }),
  unblock: (value: string) => invoke<void>('research_unblock', { value }),
  read: (paperId: number, refresh = false) => invoke<PaperReading>('research_read', { paperId, refresh }),
  cite: (paperId: number) => invoke<Citations>('research_cite', { paperId }),
  explore: (paperId: number, kind: ExploreKind) => invoke<PaperView[]>('research_explore', { paperId, kind }),
  taskBoards: () => invoke<{ id: number; name: string }[]>('research_task_boards'),
  addToTasks: (paperId: number, boardId: number) => invoke<number>('research_add_to_tasks', { paperId, boardId }),
  pdf: (paperId: number) => invoke<void>('research_pdf', { paperId }),
  savePdf: (paperId: number) => invoke<boolean>('research_save_pdf', { paperId }),
  open: (paperId: number, what: 'page' | 'pdf' | 'doi') => invoke<void>('research_open', { paperId, what })
}

/** Where the reader loads a downloaded PDF from. */
export const paperPdfUrl = (paperId: number) => convertFileSrc(String(paperId), 'paperpdf')
