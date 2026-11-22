// Shared Playwright setup for the UI checks. Point CHROME at a local Chromium
// to skip `pnpm exec playwright install chromium`.
import { chromium } from 'playwright'

export const base = process.env.RIG_URL || 'http://localhost:5199'
export const email = process.env.RIG_EMAIL || 'ada@example.com'
export const password = process.env.RIG_PASSWORD || 'correct horse 1'

export async function launch() {
  try {
    return await chromium.launch({ executablePath: process.env.CHROME || undefined })
  } catch (e) {
    console.error('Could not start Chromium. Run `pnpm exec playwright install chromium` or set CHROME.\n', e.message)
    process.exit(2)
  }
}

export async function signIn(page) {
  await page.goto(base + '/login')
  await page.fill('input[type=email]', email)
  await page.fill('input[type=password]', password)
  await page.click('button[type=submit]')
  await page.waitForURL((u) => !u.pathname.startsWith('/login'))
}
