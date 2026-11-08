// Opens kwnt's first screen in a headless browser and prints what the Get started card and one of its AI tiles show, as JSON:
//   {"found": true, "text": "<the card's visible text>", "tile": true, "tileText": "<the tile's visible text>"}
//   (found/tile: false after the wait, text/tileText: "")
// node scripts/fresh_machine_page.mjs <url> <css selector of the card> <css selector of the tile> [text the tile should come to show]
// The card renders before kwnt's engine list arrives, and a tile only says "Installed" or "Signed in" once it has. So this
// always waits for the page's own GET /api/engines answer (and a moment for React to paint it) before reading, which makes
// "the tile doesn't say Installed" mean something. With a fourth argument it also waits (up to 25 s) until the tile's
// text contains it; the text printed is whatever the tile shows at the end.
// Needs playwright-core (PW_CORE=/path/to/playwright-core/index.mjs; default: the package) and a Chromium or Chrome (CHROMIUM=...).
// Used by scripts/fresh_machine_test.py. The page keeps live connections open, so a plain "dump the DOM" never finishes.
const { chromium } = await import(process.env.PW_CORE || 'playwright-core');
const [url, cardSel, tileSel, expect] = process.argv.slice(2);
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || undefined,
  args: ['--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu'],
});
const squash = (s) => s.replace(/\s+/g, ' ').trim();
let out = { found: false, text: '', tile: false, tileText: '' };
try {
  const page = await (await browser.newContext({ viewport: { width: 1280, height: 900 } })).newPage();
  const engines = page
    .waitForResponse((r) => new URL(r.url()).pathname === '/api/engines' && r.request().method() === 'GET' && r.ok(), { timeout: 30000 })
    .catch(() => null);
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 });
  try {
    const card = page.locator(cardSel).first();
    await card.waitFor({ timeout: 25000 });
    out.found = true;
    if (await engines) await page.waitForTimeout(750);
    const tile = card.locator(tileSel).first();
    try {
      await tile.waitFor({ timeout: 10000 });
      if (expect) await tile.filter({ hasText: expect }).waitFor({ timeout: 25000 }).catch(() => {});
      out.tile = true;
      out.tileText = squash(await tile.innerText());
    } catch {
      /* no such tile */
    }
    out.text = squash(await card.innerText());
  } catch {
    /* no card */
  }
} finally {
  await browser.close();
}
console.log(JSON.stringify(out));
