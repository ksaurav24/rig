// Screenshots of dashboard pages in light and dark, for visual review.
// usage: node e2e/shots.mjs [--mobile] <path>...   (writes to e2e/shots/)
import { mkdirSync } from 'node:fs'
import { base, launch, signIn } from './browser.mjs'

const args = process.argv.slice(2)
const mobile = args.includes('--mobile')
const paths = args.filter((a) => a !== '--mobile')
const viewport = mobile ? { width: 390, height: 844 } : { width: 1280, height: 800 }
const out = new URL('./shots/', import.meta.url).pathname
mkdirSync(out, { recursive: true })

const browser = await launch()
let problems = 0
for (const scheme of ['light', 'dark']) {
  const ctx = await browser.newContext({ viewport, colorScheme: scheme, isMobile: mobile, hasTouch: mobile })
  const page = await ctx.newPage()
  const errors = []
  page.on('console', (m) => m.type() === 'error' && errors.push(m.text()))
  page.on('pageerror', (e) => errors.push(String(e)))
  await signIn(page)
  for (const p of paths) {
    errors.length = 0
    await page.goto(base + p)
    await page.waitForLoadState('networkidle')
    await page.waitForTimeout(300)
    // Anything wider than the viewport means a horizontal page scroll.
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)
    const file = `${out}${mobile ? 'm' : 'd'}${p.replace(/\W+/g, '_')}-${scheme}.png`
    await page.screenshot({ path: file, fullPage: mobile })
    const notes = [overflow > 0 ? `overflows by ${overflow}px` : '', errors.join(' | ')].filter(Boolean)
    if (notes.length) problems++
    console.log(file, notes.length ? notes.join('; ') : 'ok')
  }
  await ctx.close()
}
await browser.close()
process.exit(problems ? 1 : 0)
