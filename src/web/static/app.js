// The script of both pages. A debug build serves this file as it is on disk, so saving it
// and refreshing the page is enough to see the change; a release build embeds it. Every
// part looks for what it drives before wiring itself, so one file serves the timeline and
// the target page without asking which page loaded it.

// Follow a lane link from script: some engines ignore an <a href> that wraps SVG shapes.
const chart = document.getElementById('chart');
if (chart !== null) {
  chart.addEventListener('click', event => {
    const link = event.target instanceof Element ? event.target.closest('a') : null;
    if (link === null) return;
    event.preventDefault();
    window.location.assign(link.getAttribute('href'));
  }, true);
}

// Opens the standard message of a fault under its own row, one fault at a time. The button
// carries `aria-expanded` and the message lives in a real row, so the table stays a table
// for a screen reader; a browser without JavaScript keeps the `title`. The open fault is
// marked `expanded`, and the stylesheet joins it to its message, so the pair reads as one
// block instead of two rows.
//
// The page reloads itself every 30 s. The countdown lives here: the `meta` of the head sits
// in a `noscript`, so only a browser without the script reloads on its own, and a cause
// being read can hold the countdown instead of losing it to the reload. The banner states
// the hold across the page and offers the refresh it withholds, and the countdown starts
// again when the message closes.
(() => {
  const rows = document.querySelectorAll('tr.fault');
  const paused = document.getElementById('paused');
  const refresh = document.getElementById('paused-now');
  const seconds = Number(document.body.getAttribute('data-refresh'));
  let countdown = 0;
  const schedule = () => {
    if (!(seconds > 0)) return;
    window.clearTimeout(countdown);
    countdown = window.setTimeout(() => window.location.reload(), seconds * 1000);
  };
  schedule();
  // What the page is doing is read from the page itself, so the countdown and the banner
  // can never disagree with the message on screen.
  const hold = () => {
    const open = document.querySelector('tr.cause-row:not([hidden])') !== null;
    if (paused !== null) paused.hidden = !open;
    if (open) window.clearTimeout(countdown);
    else schedule();
  };
  if (refresh !== null) refresh.addEventListener('click', () => window.location.reload());
  const close = () => {
    for (const row of document.querySelectorAll('tr.cause-row')) row.hidden = true;
    for (const row of document.querySelectorAll('tr.fault.expanded')) {
      row.classList.remove('expanded');
    }
    for (const button of document.querySelectorAll('button.cause')) {
      button.setAttribute('aria-expanded', 'false');
    }
  };
  for (const row of rows) {
    const button = row.querySelector('button.cause');
    if (!button) continue;
    const message = document.getElementById(button.getAttribute('aria-controls'));
    if (!message) continue;
    const toggle = () => {
      const open = message.hidden;
      close();
      message.hidden = !open;
      row.classList.toggle('expanded', open);
      button.setAttribute('aria-expanded', String(open));
      hold();
    };
    row.addEventListener('click', toggle);
    // The code is part of the control: it carries `aria-expanded`, so a click or a keypress
    // on it opens the message, and the event stops before the row toggles it back.
    button.addEventListener('click', event => {
      event.stopPropagation();
      toggle();
    });
  }
})();

// Replace every instant the server wrote in UTC with the locale and the time zone of the
// visitor. The server stays on UTC; only the presentation moves.
const FORMATS = {
  datetime: { dateStyle: 'medium', timeStyle: 'medium' },
  time: { hour: '2-digit', minute: '2-digit' },
  daytime: { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' },
  date: { dateStyle: 'medium', timeZone: 'UTC' },
  day: { month: 'short', day: 'numeric', timeZone: 'UTC' },
};
// The parts of a formatted instant that belong to the date. Everything else is the time, and
// a literal belongs to the side it follows, so the two halves stay in reading order.
const DATE_PARTS = new Set(['weekday', 'era', 'year', 'month', 'day']);
// An instant as one or two unbreakable halves: the date and the time may part company when a
// cell is narrow, neither of them may ever be split. The kinds that state a day and no clock
// time, and the one that states a clock time and no day, come back as a single half.
const halves = (instant, kind) => {
  const parts = new Intl.DateTimeFormat(
    undefined,
    FORMATS[kind] ?? FORMATS.datetime
  ).formatToParts(instant);
  const halves = { date: [], time: [] };
  let side = kind === 'time' ? 'time' : 'date';
  for (const part of parts) {
    if (part.type !== 'literal') side = DATE_PARTS.has(part.type) ? 'date' : 'time';
    halves[side].push(part.value);
  }
  return [halves.date.join(''), halves.time.join('')]
    .map(half => half.trim().replace(/,$/, ''))
    .filter(half => half !== '');
};
// An instant goes into the two spans a cell can break between. An element that cannot hold
// elements — the `<text>` labels of the chart, a `<title>` tooltip — takes one plain string.
const writeInstant = (element, blocks) => {
  element.replaceChildren();
  blocks.forEach((block, blockIndex) => {
    if (blockIndex > 0) element.append(' → ');
    block.forEach((half, index) => {
      if (index > 0) element.append(' ');
      const span = document.createElement('span');
      span.className = 'nowrap';
      span.textContent = half;
      element.append(span);
    });
  });
};
for (const element of document.querySelectorAll('time[datetime], [data-datetime]')) {
  const from = new Date(element.getAttribute('data-datetime') ?? element.getAttribute('datetime'));
  if (Number.isNaN(from.getTime())) continue;
  const until = element.getAttribute('data-until');
  const kind = element.getAttribute('data-format');
  const prefix = element.getAttribute('data-prefix') ?? '';
  const suffix = element.getAttribute('data-suffix') ?? '';
  const blocks = until === null
    ? [halves(from, kind)]
    : [halves(from, 'datetime'), halves(new Date(until), 'datetime')];
  if (element instanceof HTMLTimeElement) {
    writeInstant(element, blocks);
    if (prefix !== '') element.prepend(prefix);
    if (suffix !== '') element.append(suffix);
  } else {
    element.textContent = prefix + blocks.map(block => block.join(' ')).join(' → ') + suffix;
  }
}
