import { test, expect } from '@playwright/test';

const email = () => `test-${crypto.randomUUID()}@example.com`;
async function openForm(page) {
  await page.goto('/#waitlist');
  await page.getByRole('button', { name: 'Got it' }).click();
}

test('signup persists through the real local Worker and D1; repeated signup is safe', async ({ page }) => {
  await openForm(page);
  const address = email();
  for (let i = 0; i < 2; i++) {
    await page.getByLabel('Your email address').fill(address);
    await page.locator('#waitlist-consent').check();
    await page.locator('#waitlist-form button').click();
    await expect(page.locator('#waitlist-status')).toContainText("You're on the list");
    await expect(page.getByLabel('Your email address')).toHaveValue('');
  }
});

test('required consent prevents an accidental subscription', async ({ page }) => {
  await openForm(page);
  await page.getByLabel('Your email address').fill(email());
  await page.locator('#waitlist-form button').click();
  await expect(page.locator('#waitlist-status')).toBeEmpty();
  await expect(page.locator('#waitlist-consent:invalid')).toHaveCount(1);
});

test('service failure preserves the email and allows a retry', async ({ page }) => {
  await openForm(page);
  await page.route('**/api/waitlist', route => route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ ok: false, message: 'Please try again shortly.' }) }));
  const address = email();
  await page.getByLabel('Your email address').fill(address);
  await page.locator('#waitlist-consent').check();
  await page.locator('#waitlist-form button').click();
  await expect(page.locator('#waitlist-status')).toHaveText('Please try again shortly.');
  await expect(page.getByLabel('Your email address')).toHaveValue(address);
  await expect(page.locator('#waitlist-form button')).toBeEnabled();
  await expect(page.locator('#waitlist-form')).not.toHaveAttribute('aria-busy');
});

test('network failure is visible and recoverable', async ({ page }) => {
  await openForm(page);
  await page.route('**/api/waitlist', route => route.abort());
  await page.getByLabel('Your email address').fill(email());
  await page.locator('#waitlist-consent').check();
  await page.locator('#waitlist-form button').click();
  await expect(page.locator('#waitlist-status')).toContainText('Connection trouble');
  await expect(page.locator('#waitlist-form button')).toBeEnabled();
});

test('normal HTML form succeeds with JavaScript disabled', async ({ browser }) => {
  const context = await browser.newContext({ javaScriptEnabled: false });
  const page = await context.newPage();
  await page.goto('http://127.0.0.1:8790/#waitlist');
  await page.getByLabel('Your email address').fill(email());
  await page.locator('#waitlist-consent').check();
  await page.locator('#waitlist-form button').click();
  await expect(page).toHaveURL(/\/waitlist\/thanks\/$/);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText("You're on the list");
  await context.close();
});

test('notice acknowledgement persists and can be reopened from legal navigation', async ({ page }) => {
  await page.goto('/cookies/');
  const notice = page.getByRole('region', { name: 'Privacy notice' });
  await expect(notice).toBeVisible();
  await page.getByRole('button', { name: 'Got it' }).click();
  await page.reload();
  await expect(notice).toBeHidden();
  await page.getByRole('link', { name: 'Show the privacy notice again' }).click();
  await expect(notice).toBeVisible();
  await page.getByRole('button', { name: 'Got it' }).click();
  await expect(page.getByRole('link', { name: 'Show the privacy notice again' })).toBeFocused();
});

test('blocked storage does not break the notice, theme, or form', async ({ page }) => {
  await page.addInitScript(() => {
    Storage.prototype.getItem = () => { throw new Error('blocked'); };
    Storage.prototype.setItem = () => { throw new Error('blocked'); };
  });
  await openForm(page);
  await expect(page.getByRole('region', { name: 'Privacy notice' })).toBeHidden();
  await expect(page.getByLabel('Your email address')).toBeVisible();
});

for (const colorScheme of ['light', 'dark']) {
  test(`mobile ${colorScheme}: waitlist and notice fit without horizontal overflow`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.emulateMedia({ colorScheme });
    await page.goto('/#waitlist');
    await expect(page.getByRole('region', { name: 'Privacy notice' })).toBeVisible();
    await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: `test-results/waitlist-${colorScheme}-notice.png` });
    await page.getByRole('button', { name: 'Got it' }).click();
    await page.locator('#waitlist-form').scrollIntoViewIfNeeded();
    await page.screenshot({ path: `test-results/waitlist-${colorScheme}-mobile.png` });
    await expect(page.locator('#waitlist-form button')).toBeVisible();
  });
}

test('privacy pages and blog expose legal links; no optional trackers load', async ({ page }) => {
  const thirdParties = [];
  page.on('request', req => { if (!req.url().startsWith('http://127.0.0.1:8790')) thirdParties.push(req.url()); });
  for (const path of ['/privacy/', '/cookies/', '/blog/']) {
    const result = await page.goto(path);
    expect(result.status()).toBe(200);
    await expect(page.locator('footer a[href="/privacy/"]')).toBeVisible();
    await expect(page.locator('footer a[href="/cookies/"]').first()).toBeVisible();
  }
  expect(thirdParties).toEqual([]);
  expect(await page.context().cookies()).toEqual([]);
});
