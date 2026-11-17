/**
 * A small Markdown renderer for notes. Everything is escaped first, so a note can never
 * inject markup. Each block carries `data-line` (its first source line) so a click can put
 * the caret there, and task boxes carry the line they toggle.
 */

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')

function inline(src: string): string {
  const codes: string[] = []
  let s = src.replace(/`([^`]+)`/g, (_, c) => `\u0000${codes.push(c) - 1}\u0000`)
  s = esc(s)
  s = s.replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+|mailto:[^\s)]+)\)/g, (_, t, u) => `<a href="${u}">${t}</a>`)
  s = s.replace(/(^|[\s(])(https?:\/\/[^\s<)]+)/g, (_, p, u) => `${p}<a href="${u}">${u}</a>`)
  s = s.replace(/\*\*([^*]+)\*\*|__([^_]+)__/g, (_, a, b) => `<strong>${a ?? b}</strong>`)
  s = s.replace(/(^|[^*\w])\*([^*\s][^*]*)\*(?!\w)|(^|[^_\w])_([^_\s][^_]*)_(?!\w)/g, (_, p1, a, p2, b) => `${p1 ?? p2}<em>${a ?? b}</em>`)
  s = s.replace(/~~([^~]+)~~/g, '<del>$1</del>')
  s = s.replace(/==([^=]+)==/g, '<mark>$1</mark>')
  return s.replace(/\u0000(\d+)\u0000/g, (_, i) => `<code>${esc(codes[+i])}</code>`)
}

const LIST = /^(\s*)([-*+]|\d+[.)])\s+(\[[ xX]\]\s+)?(.*)$/

export function renderMarkdown(md: string): string {
  const lines = md.split('\n')
  const out: string[] = []
  let i = 0
  while (i < lines.length) {
    const line = lines[i]
    const at = ` data-line="${i}"`
    if (!line.trim()) { i++; continue }
    // fenced code
    const fence = line.match(/^\s*(```|~~~)/)
    if (fence) {
      const start = i++
      const body: string[] = []
      while (i < lines.length && !lines[i].trimStart().startsWith(fence[1])) body.push(lines[i++])
      i++
      out.push(`<pre data-line="${start}"><code>${esc(body.join('\n'))}</code></pre>`)
      continue
    }
    const hd = line.match(/^(#{1,6})\s+(.*)$/)
    if (hd) { out.push(`<h${hd[1].length}${at}>${inline(hd[2].replace(/\s#+\s*$/, ''))}</h${hd[1].length}>`); i++; continue }
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) { out.push(`<hr${at}>`); i++; continue }
    if (/^\s*>/.test(line)) {
      const start = i
      const body: string[] = []
      while (i < lines.length && /^\s*>/.test(lines[i])) body.push(lines[i++].replace(/^\s*>\s?/, ''))
      out.push(`<blockquote data-line="${start}">${renderMarkdown(body.join('\n'))}</blockquote>`)
      continue
    }
    if (LIST.test(line)) {
      const ordered = /^\s*\d/.test(line)
      const items: string[] = []
      while (i < lines.length && LIST.test(lines[i])) {
        const [, ind, , task, text] = lines[i].match(LIST)!
        const depth = Math.min(4, Math.floor(ind.replace(/\t/g, '  ').length / 2))
        const box = task ? `<input type="checkbox" data-task="${i}"${/x/i.test(task) ? ' checked' : ''}>` : ''
        const cls = [task ? 'task' : '', task && /x/i.test(task) ? 'done' : ''].filter(Boolean).join(' ')
        items.push(`<li data-line="${i}" style="--d:${depth}"${cls ? ` class="${cls}"` : ''}>${box}<span>${inline(text)}</span></li>`)
        i++
      }
      out.push(ordered ? `<ol${at}>${items.join('')}</ol>` : `<ul${at}>${items.join('')}</ul>`)
      continue
    }
    const start = i
    const para: string[] = []
    while (i < lines.length && lines[i].trim() && !/^(#{1,6}\s|\s*>|\s*```|\s*~~~)/.test(lines[i]) && !LIST.test(lines[i])) para.push(inline(lines[i++].trim()))
    out.push(`<p data-line="${start}">${para.join('<br>')}</p>`)
  }
  return out.join('')
}

/** Ticks or unticks the task box on one source line. */
export function toggleTask(md: string, line: number): string {
  const lines = md.split('\n')
  if (lines[line] != null) lines[line] = lines[line].replace(/\[([ xX])\]/, (_, c) => (c === ' ' ? '[x]' : '[ ]'))
  return lines.join('\n')
}

/** The title shown for a note: its first non-empty line without Markdown marks. */
export function noteTitle(md: string): string {
  const line = md.split('\n').map(l => l.trim()).find(Boolean) ?? ''
  return line.replace(/^#+\s*/, '').replace(/^([-*+>]|\d+[.)])\s+/, '').replace(/^\[[ xX]\]\s+/, '').replace(/[*_~`=]/g, '').slice(0, 120)
}

/** A short plain-text glimpse of the note after its title. */
export function noteSnippet(md: string): string {
  const lines = md.split('\n').map(l => l.trim()).filter(Boolean)
  return lines.slice(1).join(' ').replace(/^#+\s*|[*_~`=#>]|\[[ xX]\]/g, '').replace(/\s+/g, ' ').slice(0, 90)
}
