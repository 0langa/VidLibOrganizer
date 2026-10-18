import { existsSync } from 'node:fs';
import { defineConfig } from '@playwright/test';
export default defineConfig({ testDir: './tests/ui', testMatch: '*.spec.mjs', workers: 1, use: { baseURL: 'http://127.0.0.1:4173', launchOptions: { executablePath: process.env.CHROMIUM_PATH || (existsSync('/usr/lib/chromium/chromium') ? '/usr/lib/chromium/chromium' : undefined) } }, webServer: { command: 'node tools/serve-ui.mjs', url: 'http://127.0.0.1:4173', reuseExistingServer: false } });
