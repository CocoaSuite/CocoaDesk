type Kid = Node | string | null | false | undefined
export function h<K extends keyof HTMLElementTagNameMap>(tag: K, attrs: Record<string, any> = {}, ...kids: (Kid | Kid[])[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag)
  for (const k in attrs) {
    const v = attrs[k]
    if (v == null || v === false) continue
    if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v)
    else if (k === 'class') el.className = v
    else if (k === 'style' && typeof v === 'object') Object.assign(el.style, v)
    else if (k in el && typeof v !== 'string') (el as any)[k] = v
    else el.setAttribute(k, v === true ? '' : String(v))
  }
  for (const c of kids.flat()) if (c != null && c !== false) el.append(c as any)
  return el
}

/** Parses "YYYY-MM-DD" as a local date. */
export function parseDate(s: string): Date {
  const [y, m, d] = s.split('-').map(Number)
  return new Date(y, m - 1, d)
}

export function isoDate(d: Date): string {
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`
}

export function addDays(s: string, n: number): string {
  const d = parseDate(s)
  d.setDate(d.getDate() + n)
  return isoDate(d)
}

const fmt = (o: Intl.DateTimeFormatOptions) => new Intl.DateTimeFormat('en-GB', o)
const dayMonth = fmt({ day: 'numeric', month: 'short' })
const dayLong = fmt({ day: 'numeric', month: 'long' })
const weekdayLong = fmt({ weekday: 'long' })
const full = fmt({ weekday: 'long', day: 'numeric', month: 'long' })

export const fmtDayMonth = (s: string) => dayMonth.format(parseDate(s))
export const fmtDayLong = (s: string) => dayLong.format(parseDate(s))
export const fmtWeekday = (s: string) => weekdayLong.format(parseDate(s))
export const fmtFull = (s: string) => full.format(parseDate(s))

export const WEEKDAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday']
export const WD_SHORT = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

/** "16 – 22 November" or "28 Dec – 3 Jan". */
export function fmtRange(a: string, b: string): string {
  const x = parseDate(a), y = parseDate(b)
  if (x.getMonth() === y.getMonth()) return `${x.getDate()} – ${dayLong.format(y)}`
  return `${dayMonth.format(x)} – ${dayMonth.format(y)}`
}

export function fmtMinutes(n: number): string {
  if (n < 60) return `${n} min`
  const hrs = Math.floor(n / 60), m = n % 60
  if (hrs >= 24) {
    const d = Math.round(hrs / 24)
    return d === 1 ? '1 day' : `${d} days`
  }
  return m ? `${hrs} h ${m} min` : `${hrs} h`
}
