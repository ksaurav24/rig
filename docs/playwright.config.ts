import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './test',
  use: { baseURL: 'http://127.0.0.1:4322' },
  webServer: {
    command: 'python3 -m http.server 4322 --bind 127.0.0.1 --directory out',
    url: 'http://127.0.0.1:4322/docs/',
    reuseExistingServer: !process.env.CI,
  },
  projects: [
    { name: 'desktop', use: { ...devices['Desktop Chrome'] } },
    {
      name: 'mobile',
      use: { ...devices['iPhone 13'], browserName: 'chromium' },
    },
  ],
});
