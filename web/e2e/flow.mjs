// The main UI flow against a running Rig: project → app → deploy → secret →
// forced error → delete, asserting the toast after each step.
import { createHmac } from 'node:crypto'
import { base, email, launch, password, signIn } from './browser.mjs'

/** RFC 6238 code for a base32 secret, `offset` seconds from now. */
function totp(secret, offset = 0) {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'
  let bits = ''
  for (const c of secret) bits += alphabet.indexOf(c).toString(2).padStart(5, '0')
  const key = Buffer.from(bits.match(/.{8}/g).map((b) => parseInt(b, 2)))
  const step = Buffer.alloc(8)
  step.writeBigUInt64BE(BigInt(Math.floor((Date.now() / 1000 + offset) / 30)))
  const d = createHmac('sha1', key).update(step).digest()
  const o = d[d.length - 1] & 15
  return String((d.readUInt32BE(o) & 0x7fffffff) % 1000000).padStart(6, '0')
}

const browser = await launch()
const page = await (await browser.newContext({ viewport: { width: 1280, height: 800 } })).newPage()
const errors = []
page.on('pageerror', (e) => errors.push(String(e)))
let failed = 0
const ok = (what) => console.log('  ok   ', what)
const fail = (what) => (failed++, console.log('  FAIL ', what))

async function toast(text, what) {
  try {
    await page.locator('[data-sonner-toast]', { hasText: text }).first().waitFor({ timeout: 15000 })
    ok(what)
  } catch {
    fail(`${what} | toasts: ${JSON.stringify(await page.locator('[data-sonner-toast]').allInnerTexts())}`)
  }
}

const name = `Flow ${Date.now() % 100000}`
// Unique per run, so leftovers of an aborted run never collide with this one.
const dbName = `flowdb${Date.now() % 100000}`
await signIn(page)
try {

await page.goto(base + '/projects')
await page.getByRole('button', { name: 'New project' }).first().click()
await page.fill('#project-name', name)
await page.getByRole('button', { name: 'Create project' }).click()
await toast('Project created', 'project created')

// Creating a project lands on it, with the ways to add a first service.
await page.waitForURL(/\/projects\/[^/]+$/)
await page.getByRole('button', { name: /^App\b/ }).click()
await page.getByRole('dialog', { name: 'New app' }).waitFor()
ok('new project lands on its page and opens the New app dialog')
await page.getByRole('dialog').getByRole('button', { name: /Docker image/ }).click()
await page.getByRole('dialog', { name: /New app from Docker image/ }).waitFor()
await page.fill('#app-name', 'flow')
await page.fill('#app-image', 'nginx:alpine')
await page.fill('#app-port', '80')
await page.getByRole('button', { name: 'Create and deploy' }).click()
await toast('Deploying now', 'app created and deploying')
await page.waitForURL(/\/apps\/[^/]+\/deployments/)
const appPath = new URL(page.url()).pathname.replace(/\/deployments$/, '')

try {
  await page.getByText('Running', { exact: true }).first().waitFor({ timeout: 90000 })
  ok('deployment reached Running')
} catch {
  fail('deployment never reached Running')
}

// A server error surfaces as an error toast carrying the server's message.
await page.route('**/api/v1/apps/*/deploy', (route) =>
  route.fulfill({
    status: 409,
    contentType: 'application/json',
    body: JSON.stringify({ error: { code: 'conflict', message: 'A deploy is already in progress.' } }),
  }),
)
await page.getByRole('button', { name: 'Redeploy' }).click()
await toast('already in progress', 'failed deploy shows an error toast')
await page.unroute('**/api/v1/apps/*/deploy')

await page.goto(base + appPath + '/environment')
await page.getByRole('button', { name: 'Add variable' }).first().click()
await page.fill('#env-key', 'FLOW_TOKEN')
await page.fill('#env-value', 's3cret-value')
// New variables default to secret; make sure it stays on.
const secretSwitch = page.getByRole('switch', { name: 'Secret' })
if ((await secretSwitch.getAttribute('aria-checked')) !== 'true') await secretSwitch.click()
const dialog = page.getByRole('dialog')
await dialog.locator('button[type=submit]').click()
await toast('Variable saved', 'variable saved')
if (await page.getByText('s3cret-value').count()) fail('secret shown in clear')
else ok('secret is masked in the list')

// A database in the same project, then a backup of it from the Backups panel.
await page.goto(base + '/databases')
await page.getByRole('button', { name: 'New database' }).first().click()
await page.fill('#db-name', dbName)
await page.locator('#db-project').click()
await page.getByRole('option', { name }).click()
await page.getByRole('dialog').locator('button[type=submit]').click()
await toast('Database starting', 'database created')
const dbRow = page.getByRole('row', { name: new RegExp(dbName) })
try {
  await dbRow.getByText('Running', { exact: true }).waitFor({ timeout: 120000 })
  ok('database reached Running')
} catch {
  fail('database never reached Running')
}
await page.getByRole('button', { name: `Actions for ${dbName}` }).click()
await page.getByRole('menuitem', { name: 'Backups' }).click()
const sheet = page.getByRole('dialog', { name: `Backups of ${dbName}` })
await sheet.getByText('No backups yet').waitFor()
ok('backups panel opens empty')
await sheet.getByRole('button', { name: 'Back up now' }).click()
await toast('Backup started', 'backup started')
try {
  await sheet.getByText('Ready', { exact: true }).waitFor({ timeout: 60000 })
  ok('backup finished')
} catch {
  fail('backup never finished')
}
const href = await sheet.getByRole('link', { name: 'Download backup' }).getAttribute('href')
const res = await page.request.get(base + href)
if (res.ok() && (await res.body()).length > 0) ok('backup downloads')
else fail(`backup download returned ${res.status()}`)
await sheet.getByRole('button', { name: 'Restore backup' }).click()
await page.getByRole('alertdialog').getByRole('button', { name: 'Restore' }).click()
await toast(`${dbName} restored`, 'backup restored')
await sheet.getByRole('button', { name: 'Delete backup' }).click()
await page.getByRole('alertdialog').getByRole('button', { name: 'Delete backup' }).click()
await toast('Backup deleted', 'backup deleted')
await sheet.getByText('No backups yet').waitFor()
await page.keyboard.press('Escape')

// Catalog: a stateless template deploys into the flow project.
await page.goto(base + '/catalog')
await page.getByLabel('Search apps').fill('it-tools')
await page.getByRole('button', { name: 'Deploy IT-Tools' }).click()
const catDialog = page.getByRole('dialog', { name: 'Deploy IT-Tools' })
await catDialog.locator('#cat-project').click()
await page.getByRole('option', { name }).click()
await catDialog.locator('#cat-name').fill('flowtools')
await catDialog.locator('button[type=submit]').click()
await toast('flowtools created. Deploying now', 'catalog app created')
try {
  await page.getByText('Running', { exact: true }).first().waitFor({ timeout: 120000 })
  ok('catalog app reached Running')
} catch {
  fail('catalog app never reached Running')
}

// Low-disk banner, with the system report faked: warn, then paused.
const GB = 1024 ** 3
const inflight = new Set()
page.on('request', (r) => inflight.add(r))
page.on('requestfinished', (r) => inflight.delete(r))
page.on('requestfailed', (r) => inflight.delete(r))
const systemReport = await (await page.request.get(base + '/api/v1/system')).json()
for (const [free, text, what] of [
  [3 * GB, 'Deploys and backups pause below 1 GB', 'low-disk banner warns'],
  [GB / 2, 'New deploys and backups are paused', 'low-disk banner says deploys are paused'],
]) {
  // Serve a canned report, so a busy server (the catalog deploy above) cannot slow the check.
  await page.route('**/api/v1/system', (route) =>
    route.fulfill({ json: { ...systemReport, disk_total: 100 * GB, disk_used: 100 * GB - free } }),
  )
  await page.goto(base + '/')
  try {
    await page.getByRole('status').filter({ hasText: text }).waitFor({ timeout: 20000 })
    ok(what)
  } catch {
    fail(what)
    console.log('    url:', page.url(), 'pending:', [...inflight].map((r) => r.url()))
    await page.screenshot({ path: '/tmp/claude-1000/-home-xlr8-code-rig/8189ff77-b91a-4c7e-82f8-8cbef5b76290/scratchpad/banner-fail.png' })
  }
  await page.unroute('**/api/v1/system')
}
await page.goto(base + '/')
await page.waitForTimeout(500)
if (await page.getByRole('status').filter({ hasText: 'free on the data disk' }).count()) fail('banner hidden with room to spare')
else ok('banner hidden with room to spare')

await page.goto(base + appPath + '/settings')
await page.getByRole('button', { name: 'Show webhook' }).click()
const hookUrl = page.locator('code', { hasText: '/api/v1/hooks/apps/' })
await hookUrl.waitFor({ timeout: 5000 })
const firstSecret = await page.locator('dd code').nth(1).textContent()
if (firstSecret && firstSecret.length >= 24) ok('webhook URL and secret shown')
else fail('webhook URL and secret shown')
await page.getByRole('button', { name: 'Rotate secret' }).click()
await toast('New webhook secret created', 'webhook secret rotated')
const secondSecret = await page.locator('dd code').nth(1).textContent()
if (secondSecret && secondSecret !== firstSecret) ok('rotated secret replaces the old one')
else fail('rotated secret replaces the old one')

await page.goto(base + appPath + '/domains')
await page.getByRole('button', { name: 'Add domain' }).first().click()
await page.fill('#domain-host', 'flow-lan.test')
await page.getByRole('combobox', { name: 'Reachable by' }).click()
await page.getByRole('option', { name: 'Local network' }).click()
await page.getByRole('combobox', { name: 'TLS' }).click()
await page.getByRole('option', { name: 'Off' }).click()
await page.getByRole('dialog').getByRole('button', { name: 'Add domain' }).click()
await toast('Domain added', 'LAN-only domain added from the dialog')
await page.getByRole('row', { name: /flow-lan\.test.*Local network/ }).waitFor()
ok('domain list shows who can reach it')

await page.goto(base + '/settings/network')
await page.getByText('You are connecting from').waitFor()
await page.getByRole('radio', { name: /Local network/ }).click()
await page.getByRole('button', { name: 'Save', exact: true }).click()
await toast('Dashboard access saved', 'dashboard limited to the LAN')
await page.getByRole('radio', { name: /Anywhere/ }).click()
await page.getByRole('button', { name: 'Save', exact: true }).click()
await toast('Dashboard access saved', 'dashboard open again')
await page.getByLabel('Domain').fill('rig.flow.test')
if (!(await page.getByRole('button', { name: 'Save address' }).isEnabled())) throw new Error('Save address stays disabled')
await page.getByLabel('Domain').fill('')
ok('dashboard address form is editable')

await page.goto(base + '/settings/github')
await page.getByRole('button', { name: 'Create GitHub App' }).waitFor()
ok('GitHub page offers to create an App')
await page.goto(base + '/settings/github/callback?code=abc123&state=forged')
await page.getByText(/expired or was not started by you/).waitFor()
ok('forged GitHub callback is refused')

await page.goto(base + '/settings/alerts')
await page.getByRole('button', { name: 'New channel' }).first().click()
await page.fill('#alert-name', 'Flow alerts')
await page.fill('#alert-url', 'http://127.0.0.1:9/flow-hook')
await page.getByRole('dialog').getByRole('button', { name: 'Add channel' }).click()
const secretDialog = page.getByRole('dialog', { name: 'Channel added' })
await secretDialog.waitFor()
if (/^[A-Za-z0-9_-]{16,}$/.test((await secretDialog.locator('code').last().textContent())?.trim() ?? '')) ok('alert signing secret shown once')
else fail('alert signing secret shown once')
await secretDialog.getByRole('button', { name: 'Done' }).click()
const alertRow = page.locator('li', { hasText: 'Flow alerts' })
await alertRow.getByRole('button', { name: 'Test' }).click()
await toast('did not accept the test alert', 'unreachable alert channel reported')
await alertRow.getByText('Last delivery failed').waitFor()
ok('delivery error kept on the channel')
await page.getByRole('switch', { name: 'Send alerts to Flow alerts' }).click()
await page.waitForFunction(() => document.querySelector('[aria-label="Send alerts to Flow alerts"]')?.getAttribute('aria-checked') === 'false')
ok('alert channel paused')
await page.getByRole('button', { name: 'Remove Flow alerts' }).click()
await page.getByRole('alertdialog').getByRole('button', { name: 'Remove channel' }).click()
await toast('Alert channel removed', 'alert channel removed')

await page.goto(base + appPath + '/settings')
await page.getByRole('button', { name: /Delete app/ }).first().click()
await page.fill('#app-delete-confirm', 'flow')
await page.getByRole('alertdialog').getByRole('button', { name: /Delete/ }).click()
await toast('App deleted', 'app deleted')
// Deleting an app returns to its project, which offers the delete.
await page.waitForURL(/\/projects\/[^/]+$/)
await page.getByRole('button', { name: 'Project actions' }).click()
await page.getByRole('menuitem', { name: 'Delete project' }).click()
await page.getByRole('alertdialog').getByRole('button', { name: /Delete/ }).click()
await toast('Project deleted', 'project deleted')

// Two-factor sign-in: turn it on, sign in with a code in a fresh browser,
// then turn it off again with a recovery code.
await page.goto(base + '/settings/account')
await page.getByRole('button', { name: 'Turn on' }).click()
await page.fill('#tfa-password', password)
await page.getByRole('button', { name: 'Continue' }).click()
await page.getByRole('img', { name: /QR code/ }).waitFor()
ok('2fa: QR code shown')
await page.getByText("Can't scan?").click()
const secret = (await page.locator('details code').innerText()).replaceAll(' ', '')
await page.fill('#tfa-code', totp(secret))
await page.getByRole('dialog').getByRole('button', { name: 'Turn on' }).click()
await toast('Two-factor sign-in is on', '2fa: turned on')
const codes = await page.getByTestId('recovery-codes').locator('li').allInnerTexts()
codes.length === 10 ? ok('2fa: ten recovery codes shown') : fail(`2fa: ${codes.length} recovery codes`)
await page.getByRole('button', { name: 'I saved them' }).click()
try {
  const other = await (await browser.newContext()).newPage()
  await other.goto(base + '/login')
  await other.fill('input[type=email]', email)
  await other.fill('input[type=password]', password)
  await other.click('button[type=submit]')
  await other.getByRole('heading', { name: 'Two-factor sign-in' }).waitFor()
  ok('2fa: login asks for a code')
  await other.fill('#code', '000000')
  await other.click('button[type=submit]')
  await other.getByText('That code did not work').waitFor()
  ok('2fa: wrong code explained')
  await other.fill('#code', totp(secret, 30))
  await other.click('button[type=submit]')
  await other.waitForURL((u) => !u.pathname.startsWith('/login'))
  ok('2fa: code signs in')
  await other.context().close()
} finally {
  // Always turn it off again so the dev account keeps working.
  await page.getByRole('button', { name: 'Turn off' }).click()
  await page.fill('#tfa-confirm-password', password)
  await page.fill('#tfa-confirm-code', codes[0] ?? '')
  await page.getByRole('dialog').getByRole('button', { name: 'Turn off' }).click()
  await toast('Two-factor sign-in is off', '2fa: turned off with a recovery code')
}

} catch (e) {
  const shot = new URL('./shots/flow-failure.png', import.meta.url).pathname
  await page.screenshot({ path: shot }).catch(() => {})
  fail(`${e.message.split('\n')[0]} (screenshot: ${shot}, at ${page.url()})`)
}

if (errors.length) fail('page errors: ' + errors.join(' | '))
else ok('no page errors')
await browser.close()
console.log(failed ? `\n${failed} check(s) failed` : '\nall UI checks passed')
process.exit(failed ? 1 : 0)
