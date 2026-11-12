// Release notes (GitHub's Markdown, as "Generate release notes" writes them) as a short list for the pages.
import { esc } from './format.js';

// "- Show the K.O. time by @someone in https://github.com/.../pull/12" -> "Show the K.O. time"
const tidy = (line) =>
  line
    .replace(/\s+by @[\w-]+ in https?:\/\/\S+$/, '')
    .replace(/\[([^\]]+)\]\([^)]+\)/g, '$1')
    .replace(/\*\*([^*]+)\*\*/g, '$1')
    .replace(/`([^`]+)`/g, '$1')
    .trim();

// the notes' bullet points, without the "Full Changelog" link and headings
export function noteItems(md) {
  return String(md || '')
    .replace(/\r/g, '')
    .split('\n')
    .filter((l) => /^\s*[-*]\s+/.test(l) && !/full changelog/i.test(l))
    .map((l) => tidy(l.replace(/^\s*[-*]\s+/, '')))
    .filter(Boolean);
}

export function notesList(md) {
  const items = noteItems(md);
  return items.length ? `<ul class="notes">${items.map((i) => `<li>${esc(i)}</li>`).join('')}</ul>` : '<div class="small">No notes for this version.</div>';
}

// CHANGELOG.md (built into the app) -> [{ version, current, lines }], newest first. current: the "(current)" one.
export function parseChangelog(text) {
  const out = [];
  let cur = null;
  for (const line of String(text || '').replace(/\r/g, '').split('\n')) {
    let m;
    if ((m = line.match(/^##\s+v?(\d[\w.-]*)\s*(\(current\))?/))) {
      cur = { version: m[1], current: !!m[2], lines: [] };
      out.push(cur);
    } else if (/^#/.test(line)) cur = null;
    else if (cur && (m = line.match(/^\s*[-*]\s+(.*)/))) cur.lines.push(tidy(m[1]));
    else if (cur && /^\s{2,}\S/.test(line) && cur.lines.length) cur.lines[cur.lines.length - 1] += ' ' + line.trim(); // a line that wraps
  }
  return out;
}

// Is version a newer than b? ("0.1.10" > "0.1.9")
export function newerVersion(a, b) {
  const pa = String(a).split(/[.-]/).map((n) => parseInt(n, 10) || 0);
  const pb = String(b).split(/[.-]/).map((n) => parseInt(n, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) if ((pa[i] || 0) !== (pb[i] || 0)) return (pa[i] || 0) > (pb[i] || 0);
  return false;
}

// The changelog's versions after `from`, up to and including `to`, newest first
export const changesSince = (changelog, from, to) =>
  parseChangelog(changelog).filter((v) => newerVersion(v.version, from) && !newerVersion(v.version, to));

const linesList = (lines) => `<ul class="notes">${lines.map((l) => `<li>${esc(l)}</li>`).join('')}</ul>`;

// versions as a timeline: each number, its lines, and "This version" on the running one
export function versionsList(versions, current) {
  if (!versions.length) return '<div class="small">No notes for this version.</div>';
  if (versions.length === 1 && versions[0].version === current) return linesList(versions[0].lines);
  return `<ol class="vtl">${versions
    .map(
      (v) =>
        `<li class="${v.version === current ? 'this' : ''}"><div class="vh"><code>${esc(v.version)}</code>${v.version === current ? '<span class="pill">This version</span>' : ''}</div>${v.lines.length ? linesList(v.lines) : '<div class="small">No notes.</div>'}</li>`,
    )
    .join('')}</ol>`;
}
