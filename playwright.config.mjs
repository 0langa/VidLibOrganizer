import { defineConfig } from '@playwright/test';
export default defineConfig({ testDir: './tests/ui', testMatch: '*.spec.mjs', workers: 1, use: { baseURL: 'http://127.0.0.1:4173', launchOptions: { executablePath: process.env.CHROMIUM_PATH || '/usr/lib/chromium/chromium' } }, webServer: { command: 'node tools/serve-ui.mjs', url: 'http://127.0.0.1:4173', reuseExistingServer: false } });
