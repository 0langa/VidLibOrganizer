const paths = {
  folder: 'M3 7V5a1 1 0 0 1 1-1h5l2 2h9a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7Zm0 2h18',
  duplicate: 'M9 9h11v11H9zM15 9V4H4v11h5',
  info: 'M12 11v6m0-10v.1M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0',
  shield: 'M12 3 4 6v6c0 5 8 9 8 9s8-4 8-9V6l-8-3Zm-4 9 3 3 5-6',
  monitor: 'M3 4h18v13H3zM8 21h8m-4-4v4',
  bolt: 'm14 2-9 12h7l-2 8 9-12h-7l2-8Z',
  fingerprint: 'M4 10a8 8 0 0 1 16 0m-12 1a4 4 0 1 1 8 0c0 5-1 8-3 11m-1-11c0 5-1 8-3 10M4 14c0 2-1 4-2 5m18-5c0 3-1 5-2 7M8 14c0 2-1 4-2 5',
  layers: 'm12 3-10 5 10 5 10-5-10-5Zm-10 9 10 5 10-5M2 16l10 5 10-5',
  scan: 'M8 3H3v5m13-5h5v5M3 16v5h5m13-5v5h-5M7 12h10',
  download: 'M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5',
  film: 'M3 3h18v18H3zM7 3v18m10-18v18M3 8h4m-4 8h4m10-8h4m-4 8h4',
  search: 'm21 21-5-5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0',
  plus: 'M12 5v14M5 12h14',
  drive: 'M5 3h14l3 13v5H2v-5L5 3Zm-3 13h20M17 18.5h.1',
  chevron: 'm9 5 7 7-7 7',
  check: 'm5 12 4 4L19 6',
  close: 'm6 6 12 12M6 18 18 6',
};
export const $ = selector => document.querySelector(selector);
export function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
export function icon(name) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('aria-hidden', 'true');
  svg.setAttribute('class', 'icon');
  const path = document.createElementNS(svg.namespaceURI, 'path');
  path.setAttribute('d', paths[name] || paths.film);
  svg.append(path);
  return svg;
}
export function hydrateIcons() {
  document.querySelectorAll('[data-icon]').forEach(node => node.replaceChildren(icon(node.dataset.icon)));
}
export function bytes(value) {
  if (!value) return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'];
  const exponent = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / (1024 ** exponent)).toLocaleString(undefined, { maximumFractionDigits: exponent ? 2 : 0 })} ${units[exponent]}`;
}
export const count = value => Number(value || 0).toLocaleString();
export const exactBytes = value => `${count(value)} bytes`;
export function displayPath(path) {
  const value = String(path ?? '');
  if (value.slice(0, 8).toUpperCase() === '\\\\?\\UNC\\') return `\\\\${value.slice(8)}`;
  if (value.startsWith('\\\\?\\')) return value.slice(4);
  return value;
}
export const basename = path => path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || path;
export function button(text, className, action, iconName) {
  const node = el('button', className);
  node.type = 'button';
  if (iconName) node.append(icon(iconName));
  node.append(document.createTextNode(text));
  node.addEventListener('click', action);
  return node;
}
