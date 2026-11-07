// Opens kwnt's first screen in a headless browser and prints what the first-run card shows, as JSON:
//   {"found": true, "text": "<the card's visible text>"}   (found: false after the wait, text: "")
// node scripts/fresh_machine_page.mjs <url> <css selector of the card> [text the card should come to show]
// The card renders before kwnt's setup status arrives and relabels its button, so with a third argument this waits
// (up to 25 s) until the card's text contains it; the text printed is whatever the card shows at the end.
// Needs playwright-core (PW_CORE=/path/to/playwright-core/index.mjs; default: the package) and a Chromium or Chrome (CHROMIUM=...).
// Used by scripts/fresh_machine_test.py. The page keeps live connections open, so a plain "dump the DOM" never finishes.
const { chromium } = await import(process.env.PW_CORE || 'playwright-core');
const [url, selector, expect] = process.argv.slice(2);
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || undefined,
  args: ['--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu'],
});
let out = { found: false, text: '' };
try {
  const page = await (await browser.newContext({ viewport: { width: 1280, height: 900 } })).newPage();
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 });
  try {
    const card = page.locator(selector).first();
    await card.waitFor({ timeout: 25000 });
    if (expect) await card.filter({ hasText: expect }).waitFor({ timeout: 25000 }).catch(() => {});
    out = { found: true, text: (await card.innerText()).replace(/\s+/g, ' ').trim() };
  } catch {
    out = { found: false, text: '' };
  }
} finally {
  await browser.close();
}
console.log(JSON.stringify(out));
