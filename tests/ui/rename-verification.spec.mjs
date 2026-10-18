import {test, expect} from '@playwright/test';
import {readFile} from 'node:fs/promises';
import {desktopBridge} from './fixtures/desktop-bridge.js';
const fixture = JSON.parse(await readFile(new URL('./fixtures/command-contracts.json', import.meta.url)));

test('resource_refusal_is_visible_and_never_authorizes_apply', async ({page}) => {
  const blocked = {...fixture, row: {...fixture.row, status: 'blocked', newName: fixture.row.oldName,
    issues: ['generated intermediate name exceeds 65536 UTF-8 bytes']}};
  await page.addInitScript(desktopBridge, blocked);
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__TAURI__.core.invoke = async (command, args) => {
      const result = await invoke(command, args);
      if (command === 'job_status' && result.result?.counts) {
        result.result.counts = {changed: '0', unchanged: '0', blocked: '1', needsReview: '0'};
      }
      return result;
    };
  });
  await page.goto('/');
  await page.getByRole('button', {name:'Rename', exact:true}).click();
  await page.getByRole('button', {name:'Choose folders', exact:true}).click();
  await page.getByLabel('Select original.mp4').check();
  await page.getByTestId('rename-preview').click();
  await expect(page.getByTestId('rename-row')).toContainText('65536 UTF-8 bytes');
  await expect(page.getByTestId('rename-apply')).toBeDisabled();
  expect(await page.evaluate(() => testBridge.calls.filter(c => c.command === 'confirm_rename' || c.command === 'apply_rename'))).toEqual([]);
});
