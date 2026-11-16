// The moodboard scene: what's on a board and the geometry to move, scale and rotate it.
// Coordinates are "world" units; the view's zoom maps them to screen pixels.

export interface Pt { x: number; y: number }
export interface Rect { x: number; y: number; w: number; h: number }

interface Base {
  id: string
  /** Top-left corner of the item's unrotated box. */
  x: number; y: number; w: number; h: number
  /** Degrees, clockwise, around the box centre. */
  rot: number
}
export interface ImageItem extends Base { type: 'image'; src: string; nw: number; nh: number }
export interface TextItem extends Base { type: 'text'; text: string; size: number; color: string }
export interface SectionItem extends Base { type: 'section'; name: string; color: string }
export type Shape = 'pen' | 'arrow' | 'rect' | 'ellipse'
export interface SketchItem extends Base {
  type: 'sketch'; shape: Shape
  /** Pen and arrow points, flat [x0, y0, x1, y1, …], relative to the box. */
  pts: number[]
  color: string; width: number
}
export type Item = ImageItem | TextItem | SectionItem | SketchItem
export interface View { x: number; y: number; zoom: number }
export interface Scene { items: Item[]; view?: View }

export const uid = () => Math.random().toString(36).slice(2, 10) + Date.now().toString(36).slice(-4)
export const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v))

const RAD = Math.PI / 180
export function rotateAround(p: Pt, c: Pt, deg: number): Pt {
  if (!deg) return { x: p.x, y: p.y }
  const a = deg * RAD, cs = Math.cos(a), sn = Math.sin(a), dx = p.x - c.x, dy = p.y - c.y
  return { x: c.x + dx * cs - dy * sn, y: c.y + dx * sn + dy * cs }
}
export const center = (it: Rect): Pt => ({ x: it.x + it.w / 2, y: it.y + it.h / 2 })
export function setCenter(it: Base, c: Pt) { it.x = c.x - it.w / 2; it.y = c.y - it.h / 2 }

/** Corners of the rotated box: top-left, top-right, bottom-right, bottom-left. */
export function corners(it: Base): Pt[] {
  const c = center(it)
  return [[it.x, it.y], [it.x + it.w, it.y], [it.x + it.w, it.y + it.h], [it.x, it.y + it.h]].map(([x, y]) => rotateAround({ x, y }, c, it.rot))
}

export function rectOf(pts: Pt[]): Rect {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity
  for (const p of pts) { x0 = Math.min(x0, p.x); y0 = Math.min(y0, p.y); x1 = Math.max(x1, p.x); y1 = Math.max(y1, p.y) }
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}
export const bounds = (it: Base): Rect => rectOf(corners(it))
export function union(rs: Rect[]): Rect | null {
  if (!rs.length) return null
  return rectOf(rs.flatMap(r => [{ x: r.x, y: r.y }, { x: r.x + r.w, y: r.y + r.h }]))
}
export const intersects = (a: Rect, b: Rect) => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
export const inside = (p: Pt, r: Rect) => p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h
export const containsRect = (outer: Rect, r: Rect) => r.x >= outer.x && r.y >= outer.y && r.x + r.w <= outer.x + outer.w && r.y + r.h <= outer.y + outer.h

/** Scales an item's size and contents by `s`, keeping its centre. */
export function scaleItem(it: Item, s: number) {
  const c = center(it)
  it.w *= s; it.h *= s
  if (it.type === 'sketch') { it.pts = it.pts.map(v => v * s); it.width *= s }
  if (it.type === 'text') it.size *= s
  setCenter(it, c)
}

/** Items that keep their proportions when resized on their own. */
export const keepsAspect = (it: Item) => it.type === 'image' || it.type === 'text' || (it.type === 'sketch' && (it.shape === 'pen' || it.shape === 'arrow'))

/** Room around pen and arrow points so the box holds the whole stroke and arrowhead. */
export const sketchPad = (it: SketchItem) => it.width * (it.shape === 'arrow' ? 5 : 1)
export const arrowHead = (width: number) => width * 4.5

/** Turns world-space pen/arrow points into an item whose box fits them. */
export function fitSketch(it: SketchItem, worldPts: number[]) {
  const pad = sketchPad(it)
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity
  for (let i = 0; i < worldPts.length; i += 2) {
    x0 = Math.min(x0, worldPts[i]); x1 = Math.max(x1, worldPts[i])
    y0 = Math.min(y0, worldPts[i + 1]); y1 = Math.max(y1, worldPts[i + 1])
  }
  it.x = x0 - pad; it.y = y0 - pad; it.w = x1 - x0 + pad * 2; it.h = y1 - y0 + pad * 2; it.rot = 0
  it.pts = worldPts.map((v, i) => v - (i % 2 ? it.y : it.x))
}

/** Ramer–Douglas–Peucker: drops points that don't change the line's shape by more than `eps`. */
export function simplify(flat: number[], eps: number): number[] {
  const n = flat.length / 2
  if (n < 3) return flat
  const keep = new Uint8Array(n); keep[0] = keep[n - 1] = 1
  const stack: [number, number][] = [[0, n - 1]]
  while (stack.length) {
    const [a, b] = stack.pop()!
    const ax = flat[a * 2], ay = flat[a * 2 + 1], bx = flat[b * 2], by = flat[b * 2 + 1]
    const dx = bx - ax, dy = by - ay, len = Math.hypot(dx, dy) || 1
    let best = -1, bestD = eps
    for (let i = a + 1; i < b; i++) {
      const d = Math.abs(dy * flat[i * 2] - dx * flat[i * 2 + 1] + bx * ay - by * ax) / len
      if (d > bestD) { bestD = d; best = i }
    }
    if (best > 0) { keep[best] = 1; stack.push([a, best], [best, b]) }
  }
  const out: number[] = []
  for (let i = 0; i < n; i++) if (keep[i]) out.push(flat[i * 2], flat[i * 2 + 1])
  return out
}

/** A smooth SVG path through freehand points. */
export function penPath(p: number[]): string {
  const n = p.length / 2
  if (n === 1) return `M${p[0]} ${p[1]}l0.01 0`
  if (n === 2) return `M${p[0]} ${p[1]}L${p[2]} ${p[3]}`
  let d = `M${p[0]} ${p[1]}`
  for (let i = 1; i < n - 1; i++) {
    const mx = (p[i * 2] + p[i * 2 + 2]) / 2, my = (p[i * 2 + 1] + p[i * 2 + 3]) / 2
    d += `Q${p[i * 2]} ${p[i * 2 + 1]} ${mx} ${my}`
  }
  return d + `L${p[n * 2 - 2]} ${p[n * 2 - 1]}`
}

export function arrowPath(p: number[], width: number): string {
  const [x0, y0, x1 = x0, y1 = y0] = p
  const a = Math.atan2(y1 - y0, x1 - x0), L = arrowHead(width), s = 0.5
  const h1x = x1 - L * Math.cos(a - s), h1y = y1 - L * Math.sin(a - s)
  const h2x = x1 - L * Math.cos(a + s), h2y = y1 - L * Math.sin(a + s)
  return `M${x0} ${y0}L${x1} ${y1}M${h1x} ${h1y}L${x1} ${y1}L${h2x} ${h2y}`
}

/** Checks a scene loaded from storage, dropping anything malformed. */
export function readScene(json: string): Scene {
  let raw: any
  try { raw = JSON.parse(json) } catch { raw = {} }
  const num = (v: any, d = 0) => (typeof v === 'number' && Number.isFinite(v) ? v : d)
  const items: Item[] = []
  for (const it of Array.isArray(raw?.items) ? raw.items : []) {
    if (!it || typeof it !== 'object' || typeof it.id !== 'string') continue
    const base = { id: it.id, x: num(it.x), y: num(it.y), w: Math.max(0.01, num(it.w, 1)), h: Math.max(0.01, num(it.h, 1)), rot: num(it.rot) }
    if (it.type === 'image' && typeof it.src === 'string') items.push({ ...base, type: 'image', src: it.src, nw: num(it.nw, base.w), nh: num(it.nh, base.h) })
    else if (it.type === 'text') items.push({ ...base, type: 'text', text: String(it.text ?? ''), size: num(it.size, 24), color: String(it.color ?? '#ececec') })
    else if (it.type === 'section') items.push({ ...base, rot: 0, type: 'section', name: String(it.name ?? 'Section'), color: String(it.color ?? '#8f8f8f') })
    else if (it.type === 'sketch' && ['pen', 'arrow', 'rect', 'ellipse'].includes(it.shape))
      items.push({ ...base, type: 'sketch', shape: it.shape, pts: Array.isArray(it.pts) ? it.pts.map((v: any) => num(v)) : [], color: String(it.color ?? '#ececec'), width: num(it.width, 2) })
  }
  const v = raw?.view
  const view = v && typeof v === 'object' ? { x: num(v.x), y: num(v.y), zoom: Math.min(64, Math.max(0.01, num(v.zoom, 1))) } : undefined
  return { items, view }
}
