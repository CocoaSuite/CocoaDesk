// The moodboard canvas: an infinite, zoomable surface with images, sections, text and sketches.
// The scene is plain data (see model.ts); the DOM is kept in step with it, one element per item.

import { h } from '../../util'
import { boardApi, boardImageUrl, errText } from '../../api'
import { toast, menuAt, type MenuItem } from '../dialogs'
import {
  type Item, type ImageItem, type TextItem, type SectionItem, type SketchItem, type Shape, type Pt, type Rect, type View,
  uid, clone, center, setCenter, corners, bounds, union, intersects, inside, containsRect, rotateAround, scaleItem,
  keepsAspect, fitSketch, simplify, penPath, arrowPath, readScene
} from './model'

export type Tool = 'select' | 'hand' | 'section' | 'text' | 'pen' | 'arrow' | 'rect' | 'ellipse' | 'eraser'
export const TOOL_KEYS: Record<string, Tool> = { v: 'select', h: 'hand', s: 'section', t: 'text', p: 'pen', a: 'arrow', r: 'rect', o: 'ellipse', e: 'eraser' }
export const COLORS: [string, string][] = [
  ['#ececec', 'White'], ['#1b1b1b', 'Black'], ['#ff6b6b', 'Red'], ['#ffa94d', 'Orange'], ['#ffd43b', 'Yellow'],
  ['#69db7c', 'Green'], ['#8AA7D9', 'Blue'], ['#b197fc', 'Purple'], ['#f783ac', 'Pink']
]
/** Stroke widths in screen pixels at the moment of drawing. */
export const WIDTHS: [number, string][] = [[2, 'Thin'], [4, 'Medium'], [8, 'Thick']]

const CLIP = 'cocoadesk-board/v1\n'
const MIN_ZOOM = 0.02, MAX_ZOOM = 32
const HISTORY = 200
const SVG = 'http://www.w3.org/2000/svg'
const isImageFile = (f: File) => f.type.startsWith('image/') || /\.(png|jpe?g|gif|webp|bmp|avif|ico)$/i.test(f.name)

type Gesture =
  | { k: 'pan'; sx: number; sy: number; vx: number; vy: number }
  | { k: 'marquee'; start: Pt; cur: Pt; base: Set<string>; body: string | null; moved: boolean }
  | { k: 'move'; start: Pt; sx: number; sy: number; ids: string[]; orig: Map<string, Pt>; started: boolean; alt: boolean }
  | { k: 'scale'; handle: number; orig: Map<string, Item>; anchor: Pt; started: boolean }
  | { k: 'rotate'; c: Pt; a0: number; orig: Map<string, Item>; started: boolean }
  | { k: 'section'; start: Pt; item: SectionItem | null }
  | { k: 'draw'; shape: Shape; start: Pt; pts: number[]; el: SVGSVGElement }
  | { k: 'erase'; last: Pt; any: boolean }

function typing(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null
  return !!el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' || el.isContentEditable)
}

function svgEl<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}): SVGElementTagNameMap[K] {
  const el = document.createElementNS(SVG, tag)
  for (const k in attrs) el.setAttribute(k, String(attrs[k]))
  return el
}

/** Draws a sketch item into an <svg> sized to its box. */
function drawSketch(svg: SVGSVGElement, it: SketchItem) {
  svg.replaceChildren()
  const style = { fill: 'none', stroke: it.color, 'stroke-width': it.width, 'stroke-linecap': 'round', 'stroke-linejoin': 'round' }
  let shape: SVGElement
  if (it.shape === 'pen') shape = svgEl('path', { d: penPath(it.pts), ...style })
  else if (it.shape === 'arrow') shape = svgEl('path', { d: arrowPath(it.pts, it.width), ...style })
  else if (it.shape === 'rect') shape = svgEl('rect', { x: 0, y: 0, width: it.w, height: it.h, rx: Math.min(it.w, it.h) * 0.03, ...style })
  else shape = svgEl('ellipse', { cx: it.w / 2, cy: it.h / 2, rx: it.w / 2, ry: it.h / 2, ...style })
  // A wider invisible copy makes thin lines easy to click and erase.
  const hit = shape.cloneNode() as SVGElement
  hit.setAttribute('class', 'hit')
  hit.setAttribute('stroke', 'transparent')
  hit.style.setProperty('--sw', it.width + 'px')
  svg.append(hit, shape)
}

export class BoardEditor {
  readonly el: HTMLElement
  private world: HTMLElement
  private secLayer: HTMLElement
  private itemLayer: HTMLElement
  private ov: SVGSVGElement
  private hint: HTMLElement
  private fileInput: HTMLInputElement

  items: Item[] = []
  view: View = { x: 0, y: 0, zoom: 1 }
  sel = new Set<string>()
  tool: Tool = 'select'
  color = COLORS[2][0]
  width = WIDTHS[1][0]

  private els = new Map<string, HTMLElement>()
  private sigs = new Map<string, string>()
  private undoStack: string[] = []
  private redoStack: string[] = []
  private g: Gesture | null = null
  private space = false
  private hover: string | null = null
  private editing: { id: string; before: string } | null = null
  private mouse: Pt | null = null
  private lastNudge = 0
  private busy = 0

  /** Called when the scene changes (to be saved) or the view moves. */
  onChange: (what: 'scene' | 'view') => void = () => {}
  /** Called when the tool, zoom or selection changes, to refresh the toolbar. */
  onState: () => void = () => {}

  constructor() {
    this.secLayer = h('div', { class: 'secs' })
    this.itemLayer = h('div', { class: 'items' })
    this.world = h('div', { class: 'world' }, this.secLayer, this.itemLayer)
    this.ov = svgEl('svg', { class: 'ov' })
    this.fileInput = h('input', { type: 'file', accept: 'image/*', multiple: true, hidden: true, onchange: () => {
      const files = [...(this.fileInput.files ?? [])]
      this.fileInput.value = ''
      void this.addFiles(files)
    } }) as HTMLInputElement
    this.hint = h('div', { class: 'bhint' },
      h('b', {}, 'Drop, paste or add images'),
      h('span', {}, 'Ctrl+V pastes screenshots and copied images. Drag pictures in from your files or a browser.'),
      h('button', { class: 'pri', onclick: () => this.pickFiles() }, 'Add images…'))
    this.el = h('div', { class: 'canvas', tabindex: '-1' }, this.world, this.ov, this.hint, this.fileInput)

    this.el.addEventListener('pointerdown', e => this.down(e))
    this.el.addEventListener('pointermove', e => this.moveEv(e))
    this.el.addEventListener('pointerup', e => this.up(e))
    this.el.addEventListener('pointercancel', e => this.up(e))
    this.el.addEventListener('pointerleave', () => { this.mouse = null; if (this.hover && !this.g) { this.hover = null; this.drawOverlay() } })
    this.el.addEventListener('dblclick', e => this.dbl(e))
    this.el.addEventListener('wheel', e => this.wheel(e), { passive: false })
    this.el.addEventListener('contextmenu', e => this.context(e))
    this.el.addEventListener('dragstart', e => e.preventDefault())
    this.el.addEventListener('dragover', e => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy'; this.el.classList.add('dropping') })
    this.el.addEventListener('dragleave', e => { if (e.target === this.el) this.el.classList.remove('dropping') })
    this.el.addEventListener('drop', e => this.drop(e))
    document.addEventListener('keydown', e => this.key(e))
    document.addEventListener('keyup', e => { if (e.code === 'Space') { this.space = false; this.cursor() } })
    window.addEventListener('blur', () => { this.space = false; this.cursor() })
    document.addEventListener('paste', e => this.paste(e))
    document.addEventListener('copy', e => this.copy(e, false))
    document.addEventListener('cut', e => this.copy(e, true))
    new ResizeObserver(() => this.drawOverlay()).observe(this.el)
  }

  /** Whether keyboard and clipboard events are meant for this board. */
  private get active() {
    return this.el.isConnected && !document.querySelector('.modal,.ctx')
  }

  // ---------- loading ----------

  load(sceneJson: string) {
    this.finishEdit()
    const scene = readScene(sceneJson)
    this.items = scene.items
    this.sel.clear(); this.hover = null; this.g = null
    this.undoStack = []; this.redoStack = []
    for (const el of this.els.values()) el.remove()
    this.els.clear(); this.sigs.clear()
    this.render()
    if (scene.view) { this.view = scene.view; this.applyView() } else requestAnimationFrame(() => this.fitAll(false))
    this.onState()
  }

  sceneJson(): string {
    const v = this.view
    return JSON.stringify({ items: this.items, view: { x: Math.round(v.x * 10) / 10, y: Math.round(v.y * 10) / 10, zoom: v.zoom } })
  }

  // ---------- coordinates and view ----------

  private local(e: { clientX: number; clientY: number }): Pt {
    const r = this.el.getBoundingClientRect()
    return { x: e.clientX - r.left, y: e.clientY - r.top }
  }
  toWorld(p: Pt): Pt { return { x: (p.x - this.view.x) / this.view.zoom, y: (p.y - this.view.y) / this.view.zoom } }
  private toScreen(p: Pt): Pt { return { x: p.x * this.view.zoom + this.view.x, y: p.y * this.view.zoom + this.view.y } }
  private worldAt(e: { clientX: number; clientY: number }) { return this.toWorld(this.local(e)) }
  private viewCenter(): Pt { return this.toWorld({ x: this.el.clientWidth / 2, y: this.el.clientHeight / 2 }) }

  private applyView() {
    const v = this.view
    this.world.style.transform = `translate(${v.x}px,${v.y}px) scale(${v.zoom})`
    this.world.style.setProperty('--z', String(v.zoom))
    // The dotted backdrop moves with the board so panning feels anchored.
    const step = 24 * Math.pow(2, Math.round(Math.log2(1 / v.zoom)))
    const px = step * v.zoom
    this.el.style.backgroundSize = `${px}px ${px}px`
    this.el.style.backgroundPosition = `${v.x}px ${v.y}px`
    this.drawOverlay()
    this.onState()
  }

  private viewChanged() { this.applyView(); this.onChange('view') }

  zoomAt(screen: Pt, zoom: number) {
    zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom))
    const w = this.toWorld(screen)
    this.view = { zoom, x: screen.x - w.x * zoom, y: screen.y - w.y * zoom }
    this.viewChanged()
  }
  zoomBy(f: number) { this.zoomAt({ x: this.el.clientWidth / 2, y: this.el.clientHeight / 2 }, this.view.zoom * f) }
  zoomTo(z: number) { this.zoomAt({ x: this.el.clientWidth / 2, y: this.el.clientHeight / 2 }, z) }

  private fitRect(r: Rect | null, save = true) {
    const cw = this.el.clientWidth, ch = this.el.clientHeight
    if (!r || !cw || !ch) { if (!r) { this.view = { x: cw / 2, y: ch / 2, zoom: 1 }; this.applyView() } return }
    const pad = Math.min(60, cw * 0.08, ch * 0.08)
    const zoom = Math.min(4, Math.max(MIN_ZOOM, Math.min((cw - pad * 2) / Math.max(r.w, 1), (ch - pad * 2) / Math.max(r.h, 1))))
    const c = center(r)
    this.view = { zoom, x: cw / 2 - c.x * zoom, y: ch / 2 - c.y * zoom }
    save ? this.viewChanged() : this.applyView()
  }
  fitAll(save = true) { this.fitRect(union(this.items.map(bounds)), save) }
  fitSelection() { this.sel.size ? this.fitRect(union(this.selected().map(bounds))) : this.fitAll() }

  // ---------- history ----------

  private snapshot() {
    this.undoStack.push(JSON.stringify(this.items))
    if (this.undoStack.length > HISTORY) this.undoStack.shift()
    this.redoStack = []
  }
  private restore(from: string[], to: string[]) {
    this.finishEdit()
    const s = from.pop()
    if (s == null) return
    to.push(JSON.stringify(this.items))
    this.items = JSON.parse(s)
    for (const id of [...this.sel]) if (!this.byId(id)) this.sel.delete(id)
    this.changed()
  }
  undo() { this.restore(this.undoStack, this.redoStack) }
  redo() { this.restore(this.redoStack, this.undoStack) }
  get canUndo() { return this.undoStack.length > 0 }
  get canRedo() { return this.redoStack.length > 0 }

  private changed() { this.render(); this.onChange('scene'); this.onState() }

  // ---------- items ----------

  byId(id: string) { return this.items.find(i => i.id === id) }
  selected(): Item[] { return this.items.filter(i => this.sel.has(i.id)) }

  /** The given items plus everything sitting inside any section among them. */
  private withChildren(ids: Iterable<string>): Set<string> {
    const out = new Set(ids)
    let grew = true
    while (grew) {
      grew = false
      for (const s of this.items) {
        if (s.type !== 'section' || !out.has(s.id)) continue
        for (const it of this.items) {
          if (!out.has(it.id) && inside(center(it), s) && !(it.type === 'section' && it.w * it.h >= s.w * s.h)) { out.add(it.id); grew = true }
        }
      }
    }
    return out
  }

  private select(ids: Iterable<string>) {
    this.sel = new Set(ids)
    this.drawOverlay(); this.onState()
  }

  setTool(t: Tool) {
    this.finishEdit()
    this.tool = t
    if (t !== 'select') this.hover = null
    this.cursor(); this.drawOverlay(); this.onState()
  }

  private cursor() {
    this.el.dataset.tool = this.space ? 'hand' : this.tool
  }

  /** Applies a colour to the selected sketches, text and sections, and to what's drawn next. */
  setColor(c: string) {
    this.color = c
    const targets = this.selected().filter(i => i.type !== 'image')
    if (targets.length) {
      this.snapshot()
      for (const it of targets) (it as TextItem | SketchItem | SectionItem).color = c
      this.changed()
    }
    this.onState()
  }

  setWidth(w: number) {
    this.width = w
    const targets = this.selected().filter((i): i is SketchItem => i.type === 'sketch')
    if (targets.length) {
      this.snapshot()
      for (const it of targets) it.width = w / this.view.zoom
      this.changed()
    }
    this.onState()
  }

  deleteSelected() {
    if (!this.sel.size) return
    this.snapshot()
    this.items = this.items.filter(i => !this.sel.has(i.id))
    this.sel.clear()
    this.changed()
  }

  /** Copies the selection (sections with their contents) and selects the copies. */
  duplicate(offset = true) {
    if (!this.sel.size) return
    this.snapshot()
    const ids = this.withChildren(this.sel)
    const d = offset ? 20 / this.view.zoom : 0
    const copies = this.items.filter(i => ids.has(i.id)).map(i => ({ ...clone(i), id: uid(), x: i.x + d, y: i.y + d }))
    this.items.push(...copies)
    this.sel = new Set(copies.map(c => c.id))
    this.changed()
    return copies
  }

  bringToFront() { this.reorder(true) }
  sendToBack() { this.reorder(false) }
  private reorder(front: boolean) {
    if (!this.sel.size) return
    this.snapshot()
    const picked = this.items.filter(i => this.sel.has(i.id)), rest = this.items.filter(i => !this.sel.has(i.id))
    this.items = front ? [...rest, ...picked] : [...picked, ...rest]
    this.changed()
  }

  selectAll() { this.select(this.items.map(i => i.id)) }

  resetRotation() {
    const its = this.selected().filter(i => i.rot)
    if (!its.length) return
    this.snapshot()
    for (const it of its) it.rot = 0
    this.changed()
  }

  originalSize() {
    const its = this.selected().filter((i): i is ImageItem => i.type === 'image')
    if (!its.length) return
    this.snapshot()
    for (const it of its) { const c = center(it); it.w = it.nw; it.h = it.nh; setCenter(it, c) }
    this.changed()
  }

  /** Puts a new section around the selection. */
  wrapInSection() {
    const r = union(this.selected().map(bounds))
    if (!r) return
    this.snapshot()
    const pad = 24 / this.view.zoom
    const s: SectionItem = { id: uid(), type: 'section', name: this.nextSectionName(), color: '#8f8f8f', x: r.x - pad, y: r.y - pad, w: r.w + pad * 2, h: r.h + pad * 2, rot: 0 }
    this.items.unshift(s)
    this.sel = new Set([s.id])
    this.changed()
  }

  private nextSectionName() {
    const n = this.items.filter(i => i.type === 'section').length + 1
    return `Section ${n}`
  }

  private nudge(dx: number, dy: number) {
    if (!this.sel.size) return
    if (Date.now() - this.lastNudge > 600) this.snapshot()
    this.lastNudge = Date.now()
    for (const id of this.withChildren(this.sel)) { const it = this.byId(id)!; it.x += dx; it.y += dy }
    this.changed()
  }

  // ---------- rendering ----------

  render() {
    const want = new Set(this.items.map(i => i.id))
    for (const [id, el] of this.els) if (!want.has(id)) { el.remove(); this.els.delete(id); this.sigs.delete(id) }
    const secs: HTMLElement[] = [], rest: HTMLElement[] = []
    for (const it of this.items) (it.type === 'section' ? secs : rest).push(this.renderItem(it))
    this.order(this.secLayer, secs)
    this.order(this.itemLayer, rest)
    this.measureText()
    this.hint.hidden = this.items.length > 0
    this.drawOverlay()
  }

  private order(layer: HTMLElement, els: HTMLElement[]) {
    els.forEach((el, i) => { if (layer.children[i] !== el) layer.insertBefore(el, layer.children[i] ?? null) })
  }

  private renderItem(it: Item): HTMLElement {
    let el = this.els.get(it.id)
    if (!el || el.dataset.type !== it.type) {
      el?.remove()
      el = this.create(it)
      this.els.set(it.id, el)
      this.sigs.delete(it.id)
    }
    const s = el.style
    s.width = it.w + 'px'
    s.height = it.h + 'px'
    s.transform = `translate(${it.x}px,${it.y}px)` + (it.rot ? ` rotate(${it.rot}deg)` : '')
    if (it.type === 'image') {
      const img = el.firstElementChild as HTMLImageElement
      if (img.dataset.src !== it.src) { img.dataset.src = it.src; el.classList.remove('missing'); img.src = boardImageUrl(it.src) }
    } else if (it.type === 'text') {
      s.width = s.height = ''
      s.fontSize = it.size + 'px'
      s.color = it.color
      if (this.editing?.id !== it.id && el.textContent !== it.text) el.textContent = it.text
    } else if (it.type === 'section') {
      s.setProperty('--c', it.color)
      const label = el.querySelector('.sl') as HTMLElement
      if (!label.isContentEditable && label.textContent !== it.name) label.textContent = it.name
    } else {
      const sig = JSON.stringify([it.shape, it.w, it.h, it.width, it.color, it.pts])
      if (this.sigs.get(it.id) !== sig) {
        this.sigs.set(it.id, sig)
        const svg = el.firstElementChild as SVGSVGElement
        svg.setAttribute('width', String(it.w)); svg.setAttribute('height', String(it.h))
        drawSketch(svg, it)
      }
    }
    return el
  }

  private create(it: Item): HTMLElement {
    const el = h('div', { class: 'it ' + it.type, 'data-id': it.id, 'data-type': it.type })
    if (it.type === 'image') {
      const img = h('img', { alt: '', draggable: false, decoding: 'async' }) as HTMLImageElement
      img.addEventListener('error', () => el.classList.add('missing'))
      el.append(img)
    } else if (it.type === 'section') {
      el.append(h('div', { class: 'sb', 'data-body': '' }), h('div', { class: 'sl' }))
    } else if (it.type === 'sketch') {
      el.append(svgEl('svg', { class: 'sk' }))
    }
    return el
  }

  /** Text boxes size themselves to their words; their box is read back after drawing. */
  private measureText() {
    for (const it of this.items) {
      if (it.type !== 'text') continue
      const el = this.els.get(it.id)
      if (!el) continue
      const w = el.offsetWidth, hh = el.offsetHeight
      if (w && hh && (Math.abs(w - it.w) > 0.5 || Math.abs(hh - it.h) > 0.5)) { it.w = w; it.h = hh }
    }
  }

  drawOverlay() {
    const ov = this.ov
    ov.replaceChildren()
    const poly = (pts: Pt[], cls: string) => {
      ov.append(svgEl('polygon', { points: pts.map(p => this.toScreen(p)).map(p => `${p.x},${p.y}`).join(' '), class: cls }))
    }
    if (this.hover && !this.sel.has(this.hover) && !this.g && this.tool === 'select') {
      const it = this.byId(this.hover)
      if (it) poly(corners(it), 'hov')
    }
    const sel = this.selected()
    for (const it of sel) poly(corners(it), 'sel')
    const g = this.g
    if (g?.k === 'marquee' && g.moved) {
      const a = this.toScreen(g.start), b = this.toScreen(g.cur)
      ov.append(svgEl('rect', { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), width: Math.abs(a.x - b.x), height: Math.abs(a.y - b.y), class: 'mq' }))
    }
    if (!sel.length || this.tool !== 'select' || this.editing || g?.k === 'move' || g?.k === 'marquee') return
    const single = sel.length === 1 ? sel[0] : null
    let box: Pt[], rot = 0
    if (single) { box = corners(single); rot = single.rot }
    else {
      const r = union(sel.map(bounds))!
      box = [{ x: r.x, y: r.y }, { x: r.x + r.w, y: r.y }, { x: r.x + r.w, y: r.y + r.h }, { x: r.x, y: r.y + r.h }]
      poly(box, 'grp')
    }
    const sb = box.map(p => this.toScreen(p))
    // Rotation handle, above the top edge.
    if (!sel.some(i => i.type === 'section')) {
      const top = { x: (sb[0].x + sb[1].x) / 2, y: (sb[0].y + sb[1].y) / 2 }
      const up = rotateAround({ x: 0, y: -24 }, { x: 0, y: 0 }, rot)
      const hp = { x: top.x + up.x, y: top.y + up.y }
      ov.append(svgEl('line', { x1: top.x, y1: top.y, x2: hp.x, y2: hp.y, class: 'rl' }))
      ov.append(svgEl('circle', { cx: hp.x, cy: hp.y, r: 5, class: 'hd hr', 'data-h': 'r' }))
    }
    const cursors = ['nwse-resize', 'nesw-resize', 'nwse-resize', 'nesw-resize']
    sb.forEach((p, i) => {
      // Pick the resize cursor that best matches the rotated corner.
      const turn = Math.round((((rot % 180) + 180) % 180) / 90)
      const r = svgEl('rect', { x: p.x - 4.5, y: p.y - 4.5, width: 9, height: 9, rx: 1.5, class: 'hd', 'data-h': i, transform: `rotate(${rot} ${p.x} ${p.y})` })
      r.style.cursor = cursors[(i + turn) % 4]
      ov.append(r)
    })
  }

  // ---------- pointer ----------

  /** What's under the pointer: an item, a section's empty body, or nothing. */
  private hit(target: EventTarget | null): { id: string; body: boolean } | null {
    const el = (target as Element | null)?.closest?.('[data-id]') as HTMLElement | null
    if (!el || !this.el.contains(el)) return null
    return { id: el.dataset.id!, body: !!(target as Element).closest('[data-body]') }
  }

  private down(e: PointerEvent) {
    if (this.editing) {
      const el = this.els.get(this.editing.id)
      if (el && el.contains(e.target as Node)) return
      this.finishEdit()
    }
    if ((e.target as Element).closest('.bhint button,.sl[contenteditable=plaintext-only]')) return
    this.el.focus({ preventScroll: true })
    const p = this.worldAt(e), lp = this.local(e)
    if (e.button === 1 || (e.button === 0 && (this.space || this.tool === 'hand'))) {
      e.preventDefault()
      this.g = { k: 'pan', sx: lp.x, sy: lp.y, vx: this.view.x, vy: this.view.y }
      this.el.classList.add('panning')
    } else if (e.button === 2) {
      const hit = this.hit(e.target)
      if (hit && !hit.body && !this.sel.has(hit.id)) this.select([hit.id])
      return
    } else if (e.button !== 0) return
    else if (this.tool === 'select') this.downSelect(e, p)
    else if (this.tool === 'section') this.g = { k: 'section', start: p, item: null }
    else if (this.tool === 'text') {
      e.preventDefault()
      const hit = this.hit(e.target), it = hit && this.byId(hit.id)
      // Back to selecting without ending the edit that just started.
      this.tool = 'select'
      this.cursor()
      if (it?.type === 'text') this.startEdit(it.id)
      else this.addText('', p, true)
      return
    } else if (this.tool === 'eraser') {
      this.g = { k: 'erase', last: lp, any: false }
      this.eraseAt(e.clientX, e.clientY)
    } else {
      const shape = this.tool as Shape
      const svg = svgEl('svg', { class: 'sk draft' })
      this.itemLayer.append(svg)
      this.g = { k: 'draw', shape, start: p, pts: [p.x, p.y], el: svg }
      this.updateDraft()
    }
    this.el.setPointerCapture(e.pointerId)
  }

  private downSelect(e: PointerEvent, p: Pt) {
    const handle = (e.target as Element).closest?.('[data-h]')?.getAttribute('data-h')
    if (handle != null) {
      const orig = new Map(this.selected().map(i => [i.id, clone(i)] as [string, Item]))
      if (handle === 'r') {
        const c = center(this.sel.size === 1 ? this.selected()[0] : union(this.selected().map(bounds))!)
        this.g = { k: 'rotate', c, a0: Math.atan2(p.y - c.y, p.x - c.x), orig, started: false }
      } else {
        const i = Number(handle)
        const sel = this.selected()
        const box = sel.length === 1 ? corners(sel[0]) : (() => { const r = union(sel.map(bounds))!; return [{ x: r.x, y: r.y }, { x: r.x + r.w, y: r.y }, { x: r.x + r.w, y: r.y + r.h }, { x: r.x, y: r.y + r.h }] })()
        this.g = { k: 'scale', handle: i, orig, anchor: box[(i + 2) % 4], started: false }
      }
      return
    }
    const hit = this.hit(e.target)
    if (!hit || hit.body) {
      this.g = { k: 'marquee', start: p, cur: p, base: e.shiftKey ? new Set(this.sel) : new Set(), body: hit?.id ?? null, moved: false }
      return
    }
    if (e.shiftKey) {
      const s = new Set(this.sel)
      s.has(hit.id) ? s.delete(hit.id) : s.add(hit.id)
      this.select(s)
      if (!s.has(hit.id)) return
    } else if (!this.sel.has(hit.id)) this.select([hit.id])
    const lp = this.local(e)
    const ids = [...this.withChildren(this.sel)]
    this.g = { k: 'move', start: p, sx: lp.x, sy: lp.y, ids, orig: new Map(ids.map(id => { const it = this.byId(id)!; return [id, { x: it.x, y: it.y }] })), started: false, alt: e.altKey }
  }

  private moveEv(e: PointerEvent) {
    const lp = this.local(e), p = this.toWorld(lp)
    this.mouse = lp
    const g = this.g
    if (!g) {
      if (this.tool === 'select') {
        const hit = this.hit(e.target)
        const id = hit && !hit.body ? hit.id : null
        if (id !== this.hover) { this.hover = id; this.drawOverlay() }
      }
      return
    }
    switch (g.k) {
      case 'pan':
        this.view = { ...this.view, x: g.vx + lp.x - g.sx, y: g.vy + lp.y - g.sy }
        this.applyView()
        break
      case 'marquee': {
        g.cur = p
        if (!g.moved && Math.hypot(p.x - g.start.x, p.y - g.start.y) * this.view.zoom < 3) return
        g.moved = true
        const r = { x: Math.min(g.start.x, p.x), y: Math.min(g.start.y, p.y), w: Math.abs(p.x - g.start.x), h: Math.abs(p.y - g.start.y) }
        const s = new Set(g.base)
        for (const it of this.items) {
          const b = bounds(it)
          if (it.type === 'section' ? containsRect(r, b) : intersects(r, b)) s.add(it.id)
        }
        this.sel = s
        this.drawOverlay(); this.onState()
        break
      }
      case 'move': {
        if (!g.started) {
          if (Math.hypot(lp.x - g.sx, lp.y - g.sy) < 3) return
          g.started = true
          this.snapshot()
          if (g.alt) {
            // Alt-drag leaves the originals and moves copies.
            const copies = this.items.filter(i => g.ids.includes(i.id)).map(i => ({ ...clone(i), id: uid() }))
            const map = new Map(g.ids.map((id, n) => [id, copies[n]?.id]))
            this.items.push(...copies)
            g.orig = new Map(copies.map(c => [c.id, { x: c.x, y: c.y }]))
            g.ids = copies.map(c => c.id)
            this.sel = new Set([...this.sel].map(id => map.get(id)!).filter(Boolean))
          }
        }
        let dx = p.x - g.start.x, dy = p.y - g.start.y
        if (e.shiftKey) Math.abs(dx) > Math.abs(dy) ? (dy = 0) : (dx = 0)
        for (const id of g.ids) { const it = this.byId(id), o = g.orig.get(id); if (it && o) { it.x = o.x + dx; it.y = o.y + dy } }
        this.render()
        break
      }
      case 'scale': this.scaleTo(g, p, e.shiftKey); break
      case 'rotate': {
        if (!g.started) { g.started = true; this.snapshot() }
        let d = (Math.atan2(p.y - g.c.y, p.x - g.c.x) - g.a0) * 180 / Math.PI
        const single = g.orig.size === 1
        // Shift snaps a single item's angle, or a group's turn, to 15° steps.
        if (e.shiftKey && !single) d = Math.round(d / 15) * 15
        for (const [id, o] of g.orig) {
          const it = this.byId(id)
          if (!it) continue
          let rot = o.rot + d
          if (e.shiftKey && single) rot = Math.round(rot / 15) * 15
          it.rot = ((rot % 360) + 360) % 360
          if (!single) setCenter(it, rotateAround(center(o), g.c, d))
        }
        this.render()
        break
      }
      case 'section': {
        const r = { x: Math.min(g.start.x, p.x), y: Math.min(g.start.y, p.y), w: Math.abs(p.x - g.start.x), h: Math.abs(p.y - g.start.y) }
        if (!g.item) {
          if (Math.max(r.w, r.h) * this.view.zoom < 4) return
          this.snapshot()
          g.item = { id: uid(), type: 'section', name: this.nextSectionName(), color: '#8f8f8f', ...r, rot: 0 }
          this.items.unshift(g.item)
        }
        Object.assign(g.item, r)
        this.render()
        break
      }
      case 'draw': {
        if (g.shape === 'pen') {
          const n = g.pts.length
          if (Math.hypot(p.x - g.pts[n - 2], p.y - g.pts[n - 1]) * this.view.zoom >= 1.5) g.pts.push(p.x, p.y)
        } else {
          let q = p
          if (e.shiftKey && g.shape === 'arrow') {
            const a = Math.round(Math.atan2(p.y - g.start.y, p.x - g.start.x) / (Math.PI / 4)) * (Math.PI / 4), len = Math.hypot(p.x - g.start.x, p.y - g.start.y)
            q = { x: g.start.x + Math.cos(a) * len, y: g.start.y + Math.sin(a) * len }
          } else if (e.shiftKey) {
            const s = Math.max(Math.abs(p.x - g.start.x), Math.abs(p.y - g.start.y))
            q = { x: g.start.x + Math.sign(p.x - g.start.x || 1) * s, y: g.start.y + Math.sign(p.y - g.start.y || 1) * s }
          }
          g.pts = [g.start.x, g.start.y, q.x, q.y]
        }
        this.updateDraft()
        break
      }
      case 'erase': {
        // Check points along the way so a fast swipe doesn't skip strokes.
        const steps = Math.max(1, Math.ceil(Math.hypot(lp.x - g.last.x, lp.y - g.last.y) / 4))
        const r = this.el.getBoundingClientRect()
        for (let i = 1; i <= steps; i++) {
          const t = i / steps
          this.eraseAt(r.left + g.last.x + (lp.x - g.last.x) * t, r.top + g.last.y + (lp.y - g.last.y) * t)
        }
        g.last = lp
        break
      }
    }
  }

  private scaleTo(g: Extract<Gesture, { k: 'scale' }>, p: Pt, shift: boolean) {
    if (!g.started) { g.started = true; this.snapshot() }
    const min = 8 / this.view.zoom
    const sx = g.handle === 1 || g.handle === 2 ? 1 : -1, sy = g.handle >= 2 ? 1 : -1
    if (g.orig.size === 1) {
      const [id, o] = [...g.orig][0]
      const it = this.byId(id)
      if (!it) return
      const d = rotateAround(p, g.anchor, -o.rot)
      const dx = d.x - g.anchor.x, dy = d.y - g.anchor.y
      Object.assign(it, clone(o))
      if (o.type === 'text' || keepsAspect(o) !== shift) {
        const s = Math.max(min / Math.max(o.w, o.h), (dx * sx * o.w + dy * sy * o.h) / (o.w * o.w + o.h * o.h))
        scaleItem(it, s)
      } else {
        it.w = Math.max(min, dx * sx)
        it.h = Math.max(min, dy * sy)
        if (it.type === 'sketch' && o.type === 'sketch' && (it.shape === 'pen' || it.shape === 'arrow')) {
          const fx = it.w / o.w, fy = it.h / o.h
          it.pts = o.pts.map((v, n) => v * (n % 2 ? fy : fx))
        }
      }
      // The opposite corner stays put; the new centre is half the new size away from it.
      const half = rotateAround({ x: sx * it.w / 2, y: sy * it.h / 2 }, { x: 0, y: 0 }, o.rot)
      setCenter(it, { x: g.anchor.x + half.x, y: g.anchor.y + half.y })
    } else {
      const r = union([...g.orig.values()].map(bounds))!
      const dx = p.x - g.anchor.x, dy = p.y - g.anchor.y
      const s = Math.max(min / Math.max(r.w, r.h), (dx * sx * r.w + dy * sy * r.h) / (r.w * r.w + r.h * r.h))
      for (const [id, o] of g.orig) {
        const it = this.byId(id)
        if (!it) continue
        Object.assign(it, clone(o))
        scaleItem(it, s)
        const c = center(o)
        setCenter(it, { x: g.anchor.x + (c.x - g.anchor.x) * s, y: g.anchor.y + (c.y - g.anchor.y) * s })
      }
    }
    this.render()
  }

  private up(e: PointerEvent) {
    const g = this.g
    if (!g) return
    this.g = null
    if (this.el.hasPointerCapture(e.pointerId)) this.el.releasePointerCapture(e.pointerId)
    this.el.classList.remove('panning')
    switch (g.k) {
      case 'pan': this.onChange('view'); break
      case 'marquee':
        if (!g.moved) {
          if (g.body) {
            const s = e.shiftKey ? new Set(this.sel) : new Set<string>()
            s.has(g.body) ? s.delete(g.body) : s.add(g.body)
            this.sel = s
          } else if (!e.shiftKey) this.sel.clear()
        }
        this.drawOverlay(); this.onState()
        break
      case 'move': case 'scale': case 'rotate':
        if (g.started) this.changed()
        else this.drawOverlay()
        break
      case 'section':
        if (g.item) { this.sel = new Set([g.item.id]); this.changed(); this.setTool('select') }
        break
      case 'draw': this.finishDraw(g); break
      case 'erase': if (g.any) this.changed(); break
    }
  }

  private updateDraft() {
    const g = this.g
    if (g?.k !== 'draw') return
    const it = this.sketchFrom(g.shape, g.pts)
    const s = g.el.style
    s.width = it.w + 'px'; s.height = it.h + 'px'
    s.transform = `translate(${it.x}px,${it.y}px)`
    g.el.setAttribute('width', String(it.w)); g.el.setAttribute('height', String(it.h))
    drawSketch(g.el, it)
  }

  private sketchFrom(shape: Shape, pts: number[]): SketchItem {
    const it: SketchItem = { id: uid(), type: 'sketch', shape, pts: [], color: this.color, width: this.width / this.view.zoom, x: 0, y: 0, w: 1, h: 1, rot: 0 }
    if (shape === 'pen' || shape === 'arrow') fitSketch(it, pts)
    else {
      const [x0, y0, x1 = x0, y1 = y0] = pts
      Object.assign(it, { x: Math.min(x0, x1), y: Math.min(y0, y1), w: Math.max(0.01, Math.abs(x1 - x0)), h: Math.max(0.01, Math.abs(y1 - y0)) })
    }
    return it
  }

  private finishDraw(g: Extract<Gesture, { k: 'draw' }>) {
    g.el.remove()
    const z = this.view.zoom
    let pts = g.pts
    if (g.shape === 'pen') pts = simplify(pts, 0.6 / z)
    else if (pts.length < 4) return
    const it = this.sketchFrom(g.shape, pts)
    if (g.shape === 'arrow' && Math.hypot(pts[2] - pts[0], pts[3] - pts[1]) * z < 6) return
    if ((g.shape === 'rect' || g.shape === 'ellipse') && Math.max(it.w, it.h) * z < 4) return
    this.snapshot()
    this.items.push(it)
    this.changed()
  }

  private eraseAt(cx: number, cy: number) {
    const g = this.g
    for (const el of document.elementsFromPoint(cx, cy)) {
      if (!(el instanceof SVGElement) || !el.classList.contains('hit')) continue
      const id = (el.closest('[data-id]') as HTMLElement | null)?.dataset.id
      if (!id) continue
      if (g?.k === 'erase' && !g.any) { g.any = true; this.snapshot() }
      this.items = this.items.filter(i => i.id !== id)
      this.sel.delete(id)
      this.render()
      return
    }
  }

  private wheel(e: WheelEvent) {
    e.preventDefault()
    const k = e.deltaMode === 1 ? 33 : e.deltaMode === 2 ? 400 : 1
    // The wheel zooms, like PureRef. Sideways scrolling (touchpads) and Shift+wheel pan.
    const touchpad = e.deltaMode === 0 && !e.ctrlKey && e.deltaX !== 0
    if (touchpad || e.shiftKey) {
      this.view = { ...this.view, x: this.view.x - (e.shiftKey && !e.deltaX ? e.deltaY : e.deltaX) * k, y: this.view.y - (e.shiftKey ? 0 : e.deltaY) * k }
      this.viewChanged()
      return
    }
    const f = Math.exp(-e.deltaY * k * (e.ctrlKey ? 0.01 : 0.0015))
    this.zoomAt(this.local(e), this.view.zoom * f)
  }

  private dbl(e: MouseEvent) {
    if (this.tool !== 'select') return
    // Pointer capture sends the double-click to the canvas, so look at what's actually under the pointer.
    const under = document.elementFromPoint(e.clientX, e.clientY)
    const label = under?.closest('.sl') as HTMLElement | null
    const hit = this.hit(under)
    if (!hit) return
    const it = this.byId(hit.id)
    if (label && it?.type === 'section') this.renameSection(it, label)
    else if (it?.type === 'text' && !hit.body) this.startEdit(it.id)
    else if (it?.type === 'image') { this.select([it.id]); this.fitSelection() }
  }

  private context(e: MouseEvent) {
    e.preventDefault()
    const at = this.worldAt(e)
    const sel = this.selected()
    const items: MenuItem[] = []
    if (sel.length) {
      items.push(
        { label: 'Duplicate', hint: 'Ctrl+D', run: () => this.duplicate() },
        { label: 'Bring to front', hint: ']', run: () => this.bringToFront() },
        { label: 'Send to back', hint: '[', run: () => this.sendToBack() })
      if (sel.length === 1 && sel[0].type === 'image') items.push({ label: 'Original size', run: () => this.originalSize() })
      if (sel.some(i => i.rot)) items.push({ label: 'Reset rotation', run: () => this.resetRotation() })
      items.push({ label: 'Put in a new section', hint: 'Ctrl+G', run: () => this.wrapInSection() }, { label: 'Zoom to selection', hint: 'Shift+2', run: () => this.fitSelection() }, 'sep', { label: 'Delete', hint: 'Del', danger: true, run: () => this.deleteSelected() })
    } else {
      items.push(
        { label: 'Add images…', run: () => this.pickFiles() },
        { label: 'Add text here', run: () => this.addText('', at, true) },
        { label: 'Fit everything', hint: 'Shift+1', run: () => this.fitAll() })
      if (this.items.length) items.push({ label: 'Select all', hint: 'Ctrl+A', run: () => this.selectAll() })
    }
    menuAt({ x: e.clientX, y: e.clientY - 4 }, items)
  }

  // ---------- text ----------

  addText(text: string, at: Pt, edit: boolean) {
    const before = JSON.stringify(this.items)
    const it: TextItem = { id: uid(), type: 'text', text, size: 20 / this.view.zoom, color: this.color, x: at.x, y: at.y - 14 / this.view.zoom, w: 1, h: 1, rot: 0 }
    if (!edit) this.snapshot()
    this.items.push(it)
    this.sel = new Set([it.id])
    this.render()
    if (edit) this.startEdit(it.id, before)
    else this.changed()
  }

  private startEdit(id: string, before = JSON.stringify(this.items)) {
    const it = this.byId(id), el = this.els.get(id)
    if (it?.type !== 'text' || !el) return
    this.editing = { id, before }
    this.sel = new Set([id])
    el.contentEditable = 'plaintext-only'
    el.classList.add('editing')
    el.oninput = () => { it.text = el.innerText.replace(/\n$/, ''); this.measureText(); this.drawOverlay() }
    el.onkeydown = ev => {
      ev.stopPropagation()
      if (ev.key === 'Escape' || (ev.key === 'Enter' && (ev.ctrlKey || ev.metaKey))) { ev.preventDefault(); this.finishEdit() }
    }
    el.onblur = () => this.finishEdit()
    el.focus()
    const range = document.createRange()
    range.selectNodeContents(el)
    const s = getSelection(); s?.removeAllRanges(); s?.addRange(range)
    this.drawOverlay(); this.onState()
  }

  finishEdit() {
    const ed = this.editing
    if (!ed) return
    this.editing = null
    const el = this.els.get(ed.id), it = this.byId(ed.id) as TextItem | undefined
    if (el) { el.contentEditable = 'false'; el.classList.remove('editing'); el.oninput = el.onkeydown = el.onblur = null }
    getSelection()?.removeAllRanges()
    if (it && !it.text.trim()) { this.items = this.items.filter(i => i.id !== ed.id); this.sel.delete(ed.id) }
    if (JSON.stringify(this.items) !== ed.before) {
      this.undoStack.push(ed.before); this.redoStack = []
      this.changed()
    } else { this.render(); this.onState() }
  }

  private renameSection(it: SectionItem, label: HTMLElement) {
    const before = it.name
    label.contentEditable = 'plaintext-only'
    label.focus()
    const range = document.createRange(); range.selectNodeContents(label)
    const s = getSelection(); s?.removeAllRanges(); s?.addRange(range)
    const done = (keep: boolean) => {
      label.onblur = label.onkeydown = null
      label.contentEditable = 'false'
      const name = label.innerText.replace(/\s+/g, ' ').trim()
      if (keep && name && name !== before) { this.snapshot(); it.name = name; this.changed() }
      else label.textContent = before
    }
    label.onkeydown = ev => {
      ev.stopPropagation()
      if (ev.key === 'Enter') { ev.preventDefault(); label.blur() }
      if (ev.key === 'Escape') { ev.preventDefault(); label.innerText = before; label.blur() }
    }
    label.onblur = () => done(true)
  }

  // ---------- keyboard and clipboard ----------

  private key(e: KeyboardEvent) {
    if (!this.active || typing(e.target)) return
    const ctrl = e.ctrlKey || e.metaKey, k = e.key.toLowerCase()
    if (e.code === 'Space') { if (!this.space) { this.space = true; this.cursor() } e.preventDefault(); return }
    let done = true
    if (ctrl && k === 'z') e.shiftKey ? this.redo() : this.undo()
    else if (ctrl && k === 'y') this.redo()
    else if (ctrl && k === 'a') this.selectAll()
    else if (ctrl && k === 'd') this.duplicate()
    else if (ctrl && k === 'g') this.wrapInSection()
    else if (ctrl && (k === '=' || k === '+')) this.zoomBy(1.25)
    else if (ctrl && k === '-') this.zoomBy(0.8)
    else if (ctrl && k === '0') this.zoomTo(1)
    else if (ctrl || e.altKey) done = false
    else if (k === 'delete' || k === 'backspace') this.deleteSelected()
    else if (k === 'escape') { if (this.tool !== 'select') this.setTool('select'); else this.select([]) }
    else if (e.code === 'Digit1' && e.shiftKey) this.fitAll()
    else if (e.code === 'Digit2' && e.shiftKey) this.fitSelection()
    else if (e.code === 'Digit0' && e.shiftKey) this.zoomTo(1)
    else if (k === 'f') this.fitSelection()
    else if (k === ']') this.bringToFront()
    else if (k === '[') this.sendToBack()
    else if (k.startsWith('arrow')) {
      const d = (e.shiftKey ? 10 : 1) / this.view.zoom
      this.nudge(k === 'arrowleft' ? -d : k === 'arrowright' ? d : 0, k === 'arrowup' ? -d : k === 'arrowdown' ? d : 0)
    } else if (!e.shiftKey && TOOL_KEYS[k]) this.setTool(TOOL_KEYS[k])
    else done = false
    if (done) e.preventDefault()
  }

  private copy(e: ClipboardEvent, cut: boolean) {
    if (!this.active || typing(e.target) || !this.sel.size || !e.clipboardData) return
    const ids = this.withChildren(this.sel)
    e.clipboardData.setData('text/plain', CLIP + JSON.stringify({ items: this.items.filter(i => ids.has(i.id)) }))
    e.preventDefault()
    if (cut) { this.sel = ids; this.deleteSelected() }
  }

  /** Where pasted things go: under the pointer if it's over the board, else the middle of the view. */
  private pastePoint(): Pt { return this.mouse ? this.toWorld(this.mouse) : this.viewCenter() }

  private paste(e: ClipboardEvent) {
    if (!this.active || typing(e.target) || !e.clipboardData) return
    const dt = e.clipboardData
    e.preventDefault()
    const at = this.pastePoint()
    const text = dt.getData('text/plain')
    if (text.startsWith(CLIP)) return this.pasteItems(text.slice(CLIP.length), at)
    const files = [...dt.files].filter(isImageFile)
    if (files.length) return void this.addFiles(files, at)
    const urls = imageUrls(dt.getData('text/html'))
    if (urls.length) return void this.addUrls(urls, at)
    const t = text.trim()
    if (/^(https?:\/\/|data:image\/)\S+$/i.test(t)) return void this.addUrls([t], at, true)
    if (t) this.addText(t, at, false)
  }

  private pasteItems(json: string, at: Pt) {
    const items = readScene(json).items
    if (!items.length) return
    this.snapshot()
    const r = union(items.map(bounds))!, c = center(r)
    for (const it of items) { it.id = uid(); it.x += at.x - c.x; it.y += at.y - c.y }
    this.items.push(...items)
    this.sel = new Set(items.map(i => i.id))
    this.changed()
  }

  private drop(e: DragEvent) {
    e.preventDefault()
    this.el.classList.remove('dropping')
    const dt = e.dataTransfer
    if (!dt) return
    const at = this.worldAt(e)
    const files = [...dt.files].filter(isImageFile)
    if (files.length) return void this.addFiles(files, at)
    const urls = imageUrls(dt.getData('text/html'))
    if (urls.length) return void this.addUrls(urls, at)
    const uri = dt.getData('text/uri-list').split(/\r?\n/).find(l => l && !l.startsWith('#')) || dt.getData('text/plain').trim()
    if (/^(https?:\/\/|data:image\/)\S+$/i.test(uri)) void this.addUrls([uri], at)
    else if (dt.files.length) toast('Only pictures can go on a board.')
  }

  // ---------- adding images ----------

  pickFiles() { this.fileInput.click() }

  private working(delta: number) {
    this.busy += delta
    this.el.classList.toggle('busy', this.busy > 0)
  }

  async addFiles(files: File[], at?: Pt) {
    files = files.filter(isImageFile)
    if (!files.length) return
    this.working(1)
    const ids: string[] = [], errors: string[] = []
    try {
      for (const f of files) {
        try { ids.push(await boardApi.addImage(new Uint8Array(await f.arrayBuffer()))) } catch (err) { errors.push(errText(err)) }
      }
      await this.place(ids, at)
    } finally { this.working(-1) }
    if (errors.length) toast(errors.length === files.length ? errors[0] : `${errors.length} of ${files.length} files couldn't be added: ${errors[0]}`, 4500)
  }

  async addUrls(urls: string[], at?: Pt, textFallback = false) {
    this.working(1)
    const ids: string[] = []
    let error = ''
    try {
      for (const u of urls.slice(0, 30)) {
        try {
          if (u.startsWith('data:')) {
            const m = /^data:image\/[\w.+-]+;base64,(.*)$/is.exec(u)
            if (!m) throw new Error('That picture format isn’t supported.')
            const bin = atob(m[1].replace(/\s/g, ''))
            const bytes = new Uint8Array(bin.length)
            for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
            ids.push(await boardApi.addImage(bytes))
          } else ids.push(await boardApi.fetchImage(u))
        } catch (err) { error = errText(err) }
      }
      await this.place(ids, at)
    } finally { this.working(-1) }
    if (!ids.length && textFallback) this.addText(urls[0], at ?? this.viewCenter(), false)
    else if (error) toast(error, 4500)
  }

  /** Lays new images out in a loose grid around `at` and selects them. */
  private async place(ids: string[], at?: Pt) {
    if (!ids.length) return
    const dims = await Promise.all(ids.map(id => new Promise<[number, number]>(res => {
      const img = new Image()
      img.onload = () => res([img.naturalWidth || 200, img.naturalHeight || 200])
      img.onerror = () => res([200, 200])
      img.src = boardImageUrl(id)
    })))
    const z = this.view.zoom
    const maxW = (this.el.clientWidth || 800) * (ids.length > 1 ? 0.35 : 0.6) / z
    const maxH = (this.el.clientHeight || 600) * (ids.length > 1 ? 0.35 : 0.6) / z
    const gap = 16 / z
    const cols = Math.ceil(Math.sqrt(ids.length))
    const made: ImageItem[] = []
    let x = 0, y = 0, rowH = 0
    ids.forEach((src, n) => {
      const [nw, nh] = dims[n]
      const s = Math.min(1, maxW / nw, maxH / nh)
      if (n && n % cols === 0) { x = 0; y += rowH + gap; rowH = 0 }
      made.push({ id: uid(), type: 'image', src, nw, nh, x, y, w: nw * s, h: nh * s, rot: 0 })
      x += nw * s + gap; rowH = Math.max(rowH, nh * s)
    })
    const r = union(made.map(bounds))!, c = center(r), target = at ?? this.viewCenter()
    for (const it of made) { it.x += target.x - c.x; it.y += target.y - c.y }
    this.snapshot()
    this.items.push(...made)
    this.sel = new Set(made.map(i => i.id))
    if (this.tool !== 'select') this.tool = 'select'
    this.cursor()
    this.changed()
  }
}

/** Picture addresses in copied or dragged web content. */
function imageUrls(html: string): string[] {
  if (!html) return []
  const doc = new DOMParser().parseFromString(html, 'text/html')
  const out: string[] = []
  for (const img of doc.querySelectorAll('img')) {
    const srcset = img.getAttribute('srcset')
    // The largest candidate in srcset is usually the best copy.
    const best = srcset?.split(',').map(s => s.trim().split(/\s+/)).filter(p => p[0]).sort((a, b) => parseFloat(b[1] || '0') - parseFloat(a[1] || '0'))[0]?.[0]
    const src = best || img.getAttribute('src') || ''
    if (/^(https?:\/\/|data:image\/)/i.test(src) && !out.includes(src)) out.push(src)
  }
  return out
}
