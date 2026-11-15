import { invoke } from '@tauri-apps/api/core'

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
