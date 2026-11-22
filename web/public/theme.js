// Runs before first paint so the page never flashes the wrong theme or accent.
// Kept as a file (not inline) so the dashboard can ship a strict script-src.
try {
  var t = localStorage.getItem('rig-theme')
  var dark = t === 'dark' || (t !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches)
  document.documentElement.classList.toggle('dark', dark)
  var accent = localStorage.getItem('rig-accent-resolved')
  if (accent && accent !== 'emerald') {
    document.documentElement.setAttribute('data-accent', accent)
  }
} catch (e) {}
