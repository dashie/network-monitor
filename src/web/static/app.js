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
// for a screen reader; a browser without JavaScript keeps the `title`.
(() => {
  const rows = document.querySelectorAll('tr.fault');
  const close = () => {
    for (const row of document.querySelectorAll('tr.cause-row')) row.hidden = true;
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
      button.setAttribute('aria-expanded', String(open));
    };
    row.addEventListener('click', toggle);
    button.addEventListener('click', event => event.stopPropagation());
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
const label = (instant, kind) =>
  new Intl.DateTimeFormat(undefined, FORMATS[kind] ?? FORMATS.datetime).format(instant);
for (const element of document.querySelectorAll('time[datetime], [data-datetime]')) {
  const from = new Date(element.getAttribute('data-datetime') ?? element.getAttribute('datetime'));
  if (Number.isNaN(from.getTime())) continue;
  const until = element.getAttribute('data-until');
  const prefix = element.getAttribute('data-prefix') ?? '';
  const suffix = element.getAttribute('data-suffix') ?? '';
  element.textContent = prefix + (until === null
    ? label(from, element.getAttribute('data-format'))
    : `${label(from, 'datetime')} → ${label(new Date(until), 'datetime')}`) + suffix;
}
