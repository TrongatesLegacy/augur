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
