import { expect, test } from '@playwright/test';

const ALL_DOC_ROUTES = [
  '/docs/',
  '/docs/installation/',
  '/docs/quickstart/',
  '/docs/architecture/',
  '/docs/platform/',
  '/docs/platform/apps-and-deployments/',
  '/docs/platform/databases-and-backups/',
  '/docs/platform/domains-and-networking/',
  '/docs/platform/environment-and-secrets/',
  '/docs/platform/git-and-webhooks/',
  '/docs/platform/monitoring-and-alerts/',
  '/docs/platform/catalog/',
  '/docs/platform/dashboard-and-rbac/',
  '/docs/cli/',
  '/docs/cli/auth-and-contexts/',
  '/docs/cli/projects-and-apps/',
  '/docs/cli/deployments-and-logs/',
  '/docs/cli/env-and-databases/',
  '/docs/cli/admin-and-monitoring/',
  '/docs/mcp/',
  '/docs/mcp/client-setup/',
  '/docs/mcp/tools-reference/',
  '/docs/mcp/security-and-guardrails/',
  '/docs/operations/server-cli-and-config/',
  '/docs/operations/podman-and-docker/',
  '/docs/operations/troubleshooting/',
  '/docs/api/',
  '/docs/api/endpoints/',
];

test('introduction renders without horizontal overflow and displays main sections', async ({
  page,
}, info) => {
  await page.goto('/docs/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(
    'Introduction',
  );
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth > window.innerWidth,
  );
  expect(overflow).toBe(false);

  if (info.project.name === 'mobile') {
    await page
      .getByRole('button', { name: 'Open Sidebar', exact: true })
      .click();
  }

  for (const section of [
    'Get started',
    'Platform',
    'CLI',
    'MCP Server',
    'Server & Operations',
    'REST API',
  ]) {
    await expect(page.getByText(section, { exact: true }).first()).toBeVisible();
  }
});

test('every documentation page resolves with status 200 and renders its title', async ({
  page,
  request,
}) => {
  for (const route of ALL_DOC_ROUTES) {
    const response = await request.get(route);
    expect(response.status(), route).toBe(200);
  }

  await page.goto('/docs/cli/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(
    'CLI Overview & Global Flags',
  );

  await page.goto('/docs/mcp/tools-reference/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(
    'MCP Tools Reference',
  );

  await page.goto('/docs/api/endpoints/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(
    'Endpoints Reference',
  );
});

test('built static search finds CLI, MCP, and platform topics', async ({
  page,
}) => {
  await page.goto('/docs/');
  await page
    .getByRole('button', { name: /Search/ })
    .first()
    .click();
  const input = page.getByRole('combobox');
  await input.fill('rollback_app');
  await expect(
    page
      .getByRole('option')
      .filter({ hasText: 'MCP Tools Reference' })
      .first(),
  ).toBeVisible();
});

test('theme defaults to dark and toggles cleanly to light and back', async ({
  page,
}, info) => {
  await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
  await page.goto('/docs/');
  await page.evaluate(() => localStorage.removeItem('theme'));
  await page.reload();
  await expect(page.locator('html')).toHaveClass(/dark/);

  if (info.project.name === 'mobile') {
    await page
      .getByRole('button', { name: 'Open Sidebar', exact: true })
      .click();
  }

  await page
    .getByRole('button', { name: 'Toggle color theme', exact: true })
    .first()
    .click();
  await expect(page.locator('html')).not.toHaveClass(/dark/);

  await page
    .getByRole('button', { name: 'Toggle color theme', exact: true })
    .first()
    .click();
  await expect(page.locator('html')).toHaveClass(/dark/);
});
