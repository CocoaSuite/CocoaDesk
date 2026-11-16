const P: Record<string, string> = {
  plus: 'M12 5v14M5 12h14',
  check: 'M5 12l5 5 9-10',
  x: 'M6 6l12 12M18 6L6 18',
  left: 'M15 6l-6 6 6 6',
  right: 'M9 6l6 6-6 6',
  upload: 'M12 15V3M7 8l5-5 5 5M4 15v5h16v-5',
  import: 'M12 3v12M7 10l5 5 5-5M4 15v5h16v-5',
  sliders: 'M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0M14 4v4M8 10v4M16 16v4',
  cap: 'M2 9l10-5 10 5-10 5zM6 11v5c3 2 9 2 12 0v-5',
  trash: 'M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3',
  clock: 'M12 3a9 9 0 1 0 0 18a9 9 0 0 0 0-18zM12 7v5l3 2',
  edit: 'M4 20h4L19 9l-4-4L4 16zM14 6l4 4',
  feed: 'M5 5a14 14 0 0 1 14 14M5 11a8 8 0 0 1 8 8M6 18h.01',
  refresh: 'M20 11a8 8 0 0 0-14.9-3M4 4v4h4M4 13a8 8 0 0 0 14.9 3M20 20v-4h-4',
  bookmark: 'M6 3h12v18l-6-4-6 4z',
  ban: 'M12 3a9 9 0 1 0 0 18a9 9 0 0 0 0-18zM5.6 5.6l12.8 12.8',
  more: 'M5 12h.01M12 12h.01M19 12h.01'
}

export function icon(name: keyof typeof P | string, cls = 'i'): SVGSVGElement {
  const ns = 'http://www.w3.org/2000/svg'
  const svg = document.createElementNS(ns, 'svg')
  svg.setAttribute('viewBox', '0 0 24 24')
  svg.setAttribute('class', cls)
  svg.setAttribute('aria-hidden', 'true')
  const p = document.createElementNS(ns, 'path')
  p.setAttribute('d', P[name] || '')
  svg.append(p)
  return svg
}

/** The CocoaDesk mark: the CocoaSuite "C" holding a card. */
export function logo(): SVGSVGElement {
  const ns = 'http://www.w3.org/2000/svg'
  const svg = document.createElementNS(ns, 'svg')
  svg.setAttribute('viewBox', '-2 -2 56 60')
  svg.setAttribute('aria-hidden', 'true')
  const c = document.createElementNS(ns, 'path')
  c.setAttribute('d', 'M44 6C39.5 1.9 34.5 0 28 0C12.5 0 0 12.5 0 28C0 43.5 12.5 56 28 56C34.5 56 39.5 54.1 44 50L36 40C33.8 42 31.3 43 28 43C19.7 43 13 36.3 13 28C13 19.7 19.7 13 28 13C31.3 13 33.8 14 36 16L44 6Z')
  c.setAttribute('fill', '#8AA7D9')
  const r = document.createElementNS(ns, 'rect')
  for (const [k, v] of Object.entries({ x: '27', y: '22', width: '25', height: '12', rx: '2', fill: '#8AA7D9' })) r.setAttribute(k, v)
  svg.append(c, r)
  return svg
}
