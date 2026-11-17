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
  paper: 'M7 3h7l5 5v13H7zM14 3v5h5M10 13h6M10 17h6',
  search: 'M11 4a7 7 0 1 0 0 14a7 7 0 0 0 0-14zM20 20l-4-4',
  copy: 'M9 9h11v11H9zM5 15H4V4h11v1',
  feed: 'M5 5a14 14 0 0 1 14 14M5 11a8 8 0 0 1 8 8M6 18h.01',
  refresh: 'M20 11a8 8 0 0 0-14.9-3M4 4v4h4M4 13a8 8 0 0 0 14.9 3M20 20v-4h-4',
  bookmark: 'M6 3h12v18l-6-4-6 4z',
  ban: 'M12 3a9 9 0 1 0 0 18a9 9 0 0 0 0-18zM5.6 5.6l12.8 12.8',
  more: 'M5 12h.01M12 12h.01M19 12h.01',
  board: 'M4 4h7v9H4zM13 4h7v5h-7zM13 11h7v9h-7zM4 15h7v5H4z',
  cursor: 'M5 3l14 7-6 2-2 6z',
  hand: 'M8 13V5.5a1.5 1.5 0 0 1 3 0V11M11 10V4a1.5 1.5 0 0 1 3 0v6M14 10V5.5a1.5 1.5 0 0 1 3 0V14a7 7 0 0 1-7 7h-.5a6 6 0 0 1-4.6-2.2L3.5 16a1.6 1.6 0 0 1 2.4-2.1L8 16',
  frame: 'M7 3v18M17 3v18M3 7h18M3 17h18',
  text: 'M5 7V5h14v2M12 5v14M9 19h6',
  pen: 'M3 17c3-6 5-6 7-2s4 4 6-1 3-5 5-3',
  arrow: 'M5 19L19 5M10 5h9v9',
  rect: 'M4 6h16v12H4z',
  ellipse: 'M12 5c5 0 9 3.1 9 7s-4 7-9 7-9-3.1-9-7 4-7 9-7z',
  eraser: 'M8 20h12M5.5 14.5l8-8a2 2 0 0 1 2.8 0l2.2 2.2a2 2 0 0 1 0 2.8L12 18H8.5zM9.5 10.5l5 5',
  undo: 'M9 14L4 9l5-5M4 9h10a6 6 0 0 1 0 12h-3',
  redo: 'M15 14l5-5-5-5M20 9H10a6 6 0 0 0 0 12h3',
  pin: 'M9 4h6l-1 6 4 4H6l4-4zM12 14v7',
  image: 'M4 5h16v14H4zM4 16l5-5 4 4 2-2 5 5M15 9h.01',
  down: 'M6 9l6 6 6-6',
  minus: 'M5 12h14',
  help: 'M12 3a9 9 0 1 0 0 18a9 9 0 0 0 0-18zM9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.6.3-1 .9-1 1.6V14M12 17h.01',
  full: 'M4 9V4h5M4 4l6 6M15 4h5v5M20 4l-6 6M20 15v5h-5M20 20l-6-6M9 20H4v-5M4 20l6-6',
  unfull: 'M10 4v6H4M10 10L4 4M14 4v6h6M14 10l6-6M14 20v-6h6M14 14l6 6M10 20v-6H4M10 14l-6 6',
  fit: 'M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5',
  note: 'M4 4h16v10l-6 6H4zM14 20v-6h6',
  sidebar: 'M4 4h16v16H4zM9 4v16',
  split: 'M4 4h16v16H4zM12 4v16M12 12h8',
  tasks: 'M4 4h16v16H4zM8 12l3 3 5-6',
  chart: 'M5 20V12M10 20V6M15 20v-9M20 20V9',
  filter: 'M4 5h16l-6 8v6l-4-2v-4z',
  gear: 'M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6zM19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3a1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5a1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8a1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1a1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5a1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z'
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
