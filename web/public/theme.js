// Runs before first paint so the page never flashes the wrong theme.
// Kept as a file (not inline) so the dashboard can ship a strict script-src.
try {
  var t = localStorage.getItem('rig-theme')
  var dark = t === 'dark' || (t !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches)
  document.documentElement.classList.toggle('dark', dark)
} catch (e) {}
