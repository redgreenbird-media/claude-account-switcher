// Panel of Claude Accounts: renders the view the app sends and forwards clicks.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const COLORS = ['#D97757', '#5B8DEF', '#3FA66B', '#A66BD9', '#D9A23F', '#3FA6A6'];
const LABELS = { five_hour: '5 Std.', seven_day: 'Woche' };
const LENGTH = { five_hour: 5 * 3600000, seven_day: 7 * 86400000 };
const DAYS = ['So', 'Mo', 'Di', 'Mi', 'Do', 'Fr', 'Sa'];
const $ = (id) => document.getElementById(id);
let view = null;

function span(ms) {
  const minutes = Math.max(1, Math.round(ms / 60000));
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ${minutes % 60} min`;
  return `${Math.floor(hours / 24)} T ${hours % 24} h`;
}

function clock(at, now) {
  const d = new Date(at);
  const hm = d.toLocaleTimeString('de-CH', { hour: '2-digit', minute: '2-digit' });
  if (d.toDateString() === new Date(now).toDateString()) return `heute ${hm}`;
  if (d.toDateString() === new Date(now + 86400000).toDateString()) return `morgen ${hm}`;
  return `${DAYS[d.getDay()]} ${d.getDate()}.${d.getMonth() + 1}. ${hm}`;
}

function initials(name, email) {
  const source = name || email.split('@')[0].replace(/[._-]/g, ' ');
  const parts = source.split(/\s+/).filter(Boolean);
  if (parts.length >= 2) return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
  return (parts[0] || '?').slice(0, 2).toUpperCase();
}

function color(email) {
  let h = 0;
  for (const c of email) h = (h * 31 + c.charCodeAt(0)) % 9973;
  return COLORS[h % COLORS.length];
}

function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === 'class') node.className = v;
    else if (k === 'style') node.setAttribute('style', v);
    else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== false) node.setAttribute(k, v === true ? '' : v);
  }
  for (const c of children.flat()) if (c != null && c !== false) node.append(c);
  return node;
}

function bar(w, now) {
  const has = w.percent !== null && w.percent !== undefined;
  const pct = has ? Math.round(Math.min(100, Math.max(0, w.percent))) : 0;
  const level = pct >= 90 ? 'bad' : pct >= 70 ? 'warn' : '';
  let reset = '';
  if (w.isReset) reset = 'zurückgesetzt';
  else if (w.resetsAt) reset = `Reset in ${span(w.resetsAt - now)} · ${clock(w.resetsAt, now)}`;
  // Without a reset time the window still ends at most one window length after the reading.
  else if (has && w.at && LENGTH[w.kind]) reset = `Reset spätestens ${clock(w.at + LENGTH[w.kind], now)}`;
  return el('div', { class: 'bar' },
    el('span', { class: 'label' }, LABELS[w.kind] || w.kind),
    el('div', { class: 'track' }, el('div', { class: `fill ${level}`, style: `width:${pct}%` })),
    el('span', { class: 'value' }, has ? `${pct} %` : '–'),
    reset ? el('span', { class: 'reset' }, reset) : null,
  );
}

function card(a, busy) {
  const title = a.name || a.email.split('@')[0];
  const action = a.isCurrent
    ? el('span', { class: 'badge' }, 'Aktiv')
    : a.hasProfile
      ? el('button', { class: 'primary', disabled: busy, onclick: () => run(invoke('switch_account', { id: a.id })) }, 'Wechseln')
      : el('span', {});
  const age = a.measuredAt
    ? (view.now - a.measuredAt < 120000 ? 'Stand: gerade eben' : `Stand: vor ${span(view.now - a.measuredAt)}`)
    : 'Noch keine Limits – erscheinen, sobald Claude Desktop mit diesem Account läuft.';
  let term = null;
  if (a.terminal) {
    const same = a.terminal.toLowerCase() === a.email.toLowerCase();
    term = el('span', { class: same ? 'ok' : 'mismatch', title: same ? 'Im Terminal angemeldet' : `Im Terminal ist ${a.terminal} angemeldet` }, same ? '✓' : '!');
  }
  return el('article', { class: `card${a.isCurrent ? ' current' : ''}` },
    el('div', { class: 'top' },
      el('div', { class: 'avatar', style: `background:${color(a.email)}` }, initials(a.name, a.email)),
      el('div', { class: 'who' }, el('b', {}, title), el('span', {}, a.email)),
      action,
    ),
    el('div', { class: 'bars' }, a.windows.map((w) => bar(w, view.now))),
    el('div', { class: 'foot' },
      el('span', { class: 'age' }, age),
      el('button', {
        title: a.terminal ? 'Claude Code mit diesem Account in einem Terminal öffnen' : 'Claude Code mit diesem Account in einem Terminal öffnen – beim ersten Mal mit /login anmelden',
        onclick: () => run(invoke('open_terminal', { id: a.id })),
      }, term, 'Terminal'),
    ),
  );
}

function render() {
  if (!view) return;
  const busy = !!view.busy;
  const notice = $('notice');
  notice.hidden = !(view.busy || view.error);
  notice.textContent = view.busy || (view.error ? `Wechsel fehlgeschlagen: ${view.error}` : '');
  notice.className = `notice${view.error && !view.busy ? ' error' : ''}`;
  const list = $('list');
  list.replaceChildren();
  if (!view.found) list.append(el('p', { class: 'empty' }, 'Claude Desktop nicht gefunden.'));
  else if (view.accounts.length === 0) list.append(el('p', { class: 'empty' }, 'In Claude Desktop ist niemand angemeldet. Melde dich an, dann erscheint der Account hier.'));
  else for (const a of view.accounts) list.append(card(a, busy));
  $('add').disabled = busy || !view.found;
  requestAnimationFrame(() => invoke('fit_panel', { height: document.querySelector('.panel').getBoundingClientRect().height }));
}

async function run(promise) {
  try {
    await promise;
  } catch (e) {
    view = { ...view, error: String(e) };
    render();
  }
}

async function load() {
  view = await invoke('get_view');
  render();
}

$('close').addEventListener('click', () => invoke('hide_panel'));
$('add').addEventListener('click', () => run(invoke('add_account')));
document.addEventListener('keydown', (e) => { if (e.key === 'Escape') invoke('hide_panel'); });
listen('view', (e) => { view = e.payload; render(); });
listen('shown', load);
setInterval(() => { if (view) { view = { ...view, now: Date.now() }; render(); } }, 60000);
load();