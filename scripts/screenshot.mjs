import { chromium } from 'playwright';

const url = process.argv[2];
const output = process.argv[3] || '/tmp/screenshot.png';
const width = parseInt(process.argv[4] || '1280');
const height = parseInt(process.argv[5] || '720');
const fullPage = process.argv[6] === 'true';
const selector = process.argv[7] || null;

if (!url) {
  console.error('Usage: node screenshot.mjs <url> [output] [width] [height] [fullPage] [selector]');
  process.exit(1);
}

const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width, height } });
  await page.goto(url, { waitUntil: 'networkidle', timeout: 30000 });

  const screenshotOptions = { path: output };
  if (selector) {
    const element = await page.locator(selector);
    await element.screenshot(screenshotOptions);
  } else {
    screenshotOptions.fullPage = fullPage;
    await page.screenshot(screenshotOptions);
  }

  console.log(JSON.stringify({ success: true, path: output }));
} finally {
  await browser.close();
}
