// Notia's Playwright process: the browser of the host PC for the Owner's
// agent. The Rust adapter (`browser.rs`) writes it to the app data folder,
// starts it with `node` and sends it one JSON request per line on stdin; it
// answers one JSON line per request on stdout with the page after the
// action. Validation, limits and every text the model reads live in Rust
// (`backend::browser`); this file only drives Playwright.
//
// Arguments: the profile folder and `headed` or `headless`. It ends when
// stdin closes (Notia ended), on `close`, or when the person closes the
// browser window.

import readline from 'node:readline';
import { chromium } from 'playwright-core';

const [profileDir, mode] = process.argv.slice(2);
const NAVIGATION_TIMEOUT_MS = 45_000;
const ACTION_TIMEOUT_MS = 15_000;
const SETTLE_TIMEOUT_MS = 5_000;
const SETTLE_DELAY_MS = 400;
const GRID_OVERLAY_ID = '__notia_grid__';
const POINT_PAUSE_MS = 250;
const DRAG_STEPS = 25;

let context = null;
let current = null;
let closing = false;
let acceptNextDialog = false;
const dialogs = [];
const notes = [];

function send(answer) {
  process.stdout.write(`${JSON.stringify(answer)}\n`);
}

// Chrome if the PC has it, Edge otherwise (every Windows has it), and the
// Chromium of Playwright as a last resort.
async function launch() {
  const failures = [];
  for (const channel of ['chrome', 'msedge', undefined]) {
    try {
      return await chromium.launchPersistentContext(profileDir, {
        channel,
        headless: mode === 'headless',
        viewport: mode === 'headless' ? { width: 1280, height: 800 } : null,
        acceptDownloads: false,
        // Playwright turns Chrome's sandbox off unless asked; the agent
        // opens pages nobody vetted, so it stays on.
        chromiumSandbox: true,
      });
    } catch (error) {
      failures.push(`${channel ?? 'chromium'}: ${String(error?.message ?? error).split('\n')[0]}`);
    }
  }
  throw new Error(`No se pudo abrir Chrome ni Edge en la PC host (${failures.join('; ')}).`);
}

function watch(page) {
  page.on('dialog', (dialog) => {
    // An alert or leaving a page only has «Aceptar»; a confirm or a prompt
    // is accepted only when the click asked for it.
    const accept = dialog.type() === 'alert' || dialog.type() === 'beforeunload' || acceptNextDialog;
    dialogs.push(`${dialog.type()}: ${dialog.message()} (${accept ? 'aceptado' : 'cancelado'})`);
    (accept ? dialog.accept() : dialog.dismiss()).catch(() => {});
  });
  page.on('close', () => {
    if (current === page) {
      current = context?.pages().at(-1) ?? null;
    }
  });
}

async function page() {
  if (!context) {
    context = await launch();
    context.setDefaultTimeout(ACTION_TIMEOUT_MS);
    context.setDefaultNavigationTimeout(NAVIGATION_TIMEOUT_MS);
    context.on('close', () => {
      if (!closing) process.exit(0);
    });
    // A link that opens a tab leaves it active, as for the person.
    context.on('page', (opened) => {
      watch(opened);
      current = opened;
    });
    context.pages().forEach(watch);
    current = context.pages()[0] ?? null;
  }
  if (!current || current.isClosed()) {
    current = context.pages().at(-1) ?? (await context.newPage());
  }
  return current;
}

function element(target, ref) {
  return target.locator(`aria-ref=${ref}`);
}

// The role and name of a snapshot line or of how Rust described an
// element: `button "Enviar"` → ['button', 'Enviar'].
function identity(text) {
  const match = text.trim().replace(/^- /, '').match(/^([a-z]+) "((?:[^"\\]|\\.)*)"/);
  return match ? [match[1], match[2]] : null;
}

// The element `described` names on the page as it is now, when exactly one
// has that role and name: the page drew it again with another reference.
async function relocated(target, ref, described) {
  const wanted = described ? identity(described) : null;
  if (!wanted) return null;
  const snapshot = await target.ariaSnapshot({ mode: 'ai' }).catch(() => '');
  const refs = snapshot
    .split('\n')
    .filter((line) => {
      const found = identity(line);
      return found && found[0] === wanted[0] && found[1] === wanted[1];
    })
    .map((line) => line.match(/\[ref=(\w+)\]/)?.[1])
    .filter(Boolean);
  if (refs.length !== 1) return null;
  if (refs[0] !== ref) notes.push(`La referencia ${ref} ya no estaba en la página: se usó ${refs[0]}, el mismo ${described}.`);
  return element(target, refs[0]);
}

// An element of the last snapshot that is still on the page, or the same
// one drawn again; otherwise it fails right away instead of waiting for the
// action's timeout.
async function present(target, ref, described) {
  const found = element(target, ref);
  const count = await found.count().catch(() => 0);
  if (count > 0) return found;
  const again = await relocated(target, ref, described);
  if (again) return again;
  throw new Error(`la referencia ${ref} ya no está en la página (cambió desde la última instantánea): pedí browser_snapshot y usá una de las nuevas.`);
}

// Runs `action` on the element of `request`. A page that draws the element
// again while Playwright waits for it (a banner, a menu that renders twice)
// makes the action time out; it is tried once more on the element found
// again by its role and name.
async function act(target, request, action) {
  const found = await present(target, request.ref, request.described);
  try {
    return await action(found);
  } catch (error) {
    if (!/Timeout|detached|not attached/i.test(String(error?.message ?? error))) throw error;
    const again = await relocated(target, request.ref, request.described);
    if (!again) throw error;
    notes.push('El elemento cambió mientras se esperaba: se intentó una vez más.');
    return action(again);
  }
}

// A Flutter web app draws on a canvas and exposes its fields only after its
// «Enable accessibility» placeholder is pressed, so the snapshot showed
// nothing else; it is pressed here, once per page address.
const accessibleAt = new WeakMap();
async function enableAccessibility(target) {
  if (accessibleAt.get(target) === target.url()) return;
  const placeholder = target.locator('flt-semantics-placeholder');
  if ((await placeholder.count().catch(() => 0)) === 0) return;
  accessibleAt.set(target, target.url());
  await placeholder.first().dispatchEvent('click').catch(() => {});
  await target.waitForTimeout(SETTLE_DELAY_MS).catch(() => {});
  notes.push('La página es una app de Flutter: se activó su accesibilidad para ver los campos.');
}

// What a screenshot covers, in viewport pixels: one element (scrolled into
// view, cut to the viewport) or the whole viewport.
async function area(target, ref) {
  const view = await target.evaluate(() => ({ width: window.innerWidth, height: window.innerHeight }));
  if (!ref) return { x: 0, y: 0, ...view };
  const found = await present(target, ref);
  await found.scrollIntoViewIfNeeded();
  const box = await found.boundingBox();
  if (!box) throw new Error(`la referencia ${ref} no se ve en la página.`);
  const x = Math.max(0, box.x);
  const y = Math.max(0, box.y);
  const width = Math.min(box.x + box.width, view.width) - x;
  const height = Math.min(box.y + box.height, view.height) - y;
  if (width < 1 || height < 1) throw new Error(`la referencia ${ref} no se ve en la página.`);
  return { x, y, width, height };
}

// The cells over `box` with the sizes Rust sends; Rust reads the labels.
function gridOver(box, { maxCells, minCell }) {
  const cell = Math.max(minCell, Math.ceil(Math.max(box.width, box.height) / maxCells));
  return { ...box, cell, columns: Math.ceil(box.width / cell), rows: Math.ceil(box.height / cell) };
}

// Draws the grid over the page for the screenshot: rows by letter from the
// top, columns by number from the left. It takes no clicks and goes away
// right after.
async function drawGrid(target, grid) {
  await target.evaluate(
    ({ id, grid }) => {
      const overlay = document.createElement('div');
      overlay.id = id;
      overlay.setAttribute('aria-hidden', 'true');
      Object.assign(overlay.style, {
        position: 'fixed',
        left: `${grid.x}px`,
        top: `${grid.y}px`,
        width: `${grid.width}px`,
        height: `${grid.height}px`,
        overflow: 'hidden',
        pointerEvents: 'none',
        zIndex: '2147483647',
      });
      const size = Math.max(9, Math.min(14, Math.floor(grid.cell / 4)));
      for (let row = 0; row < grid.rows; row += 1) {
        for (let column = 0; column < grid.columns; column += 1) {
          const cell = document.createElement('div');
          Object.assign(cell.style, {
            position: 'absolute',
            left: `${column * grid.cell}px`,
            top: `${row * grid.cell}px`,
            width: `${grid.cell}px`,
            height: `${grid.cell}px`,
            boxSizing: 'border-box',
            border: '1px solid rgba(255, 0, 90, 0.9)',
          });
          const label = document.createElement('span');
          label.textContent = `${String.fromCharCode(65 + row)}${column + 1}`;
          Object.assign(label.style, {
            position: 'absolute',
            left: '1px',
            top: '1px',
            padding: '0 2px',
            font: `bold ${size}px sans-serif`,
            lineHeight: '1.2',
            color: '#ffffff',
            background: 'rgba(255, 0, 90, 0.9)',
          });
          cell.appendChild(label);
          overlay.appendChild(cell);
        }
      }
      document.documentElement.appendChild(overlay);
    },
    { id: GRID_OVERLAY_ID, grid },
  );
}

async function removeGrid(target) {
  await target.evaluate((id) => document.getElementById(id)?.remove(), GRID_OVERLAY_ID).catch(() => {});
}

async function screenshot(target, request) {
  const box = await area(target, request.ref);
  const grid = request.grid ? gridOver(box, request.grid) : null;
  if (grid) await drawGrid(target, grid);
  try {
    const clip = request.ref ? { clip: box } : {};
    const image = await target.screenshot({ type: 'jpeg', quality: 70, scale: 'css', ...clip });
    return { image: image.toString('base64'), grid };
  } finally {
    if (grid) await removeGrid(target);
  }
}

async function settle(target) {
  await target.waitForTimeout(SETTLE_DELAY_MS).catch(() => {});
  await target.waitForLoadState('domcontentloaded', { timeout: SETTLE_TIMEOUT_MS }).catch(() => {});
  await enableAccessibility(target);
}

async function tabs() {
  const pages = context.pages();
  return Promise.all(
    pages.map(async (tab, index) => ({
      index,
      url: tab.url(),
      title: await tab.title().catch(() => ''),
      active: tab === current,
    })),
  );
}

async function state(target, { ref, withSnapshot = true } = {}) {
  let snapshot = '';
  if (ref) snapshot = await (await present(target, ref)).ariaSnapshot({ mode: 'ai' });
  else if (withSnapshot) snapshot = await target.ariaSnapshot({ mode: 'ai' }).catch(() => '');
  return {
    url: target.url(),
    title: await target.title().catch(() => ''),
    tabs: await tabs(),
    snapshot,
    dialogs: dialogs.splice(0),
    notes: notes.splice(0),
    signIn: await asksToSignIn(target),
  };
}

// A visible password field, in the page or one of its frames, means the
// site asks to sign in: the profile's session is not active there.
async function asksToSignIn(target) {
  for (const frame of target.frames()) {
    const count = await frame.locator('input[type="password"]:visible').count().catch(() => 0);
    if (count > 0) return true;
  }
  return false;
}

async function run(request) {
  if (request.op === 'close') {
    closing = true;
    await context?.close().catch(() => {});
    context = null;
    return {};
  }
  let target = await page();
  switch (request.op) {
    case 'navigate':
      if (request.url) await target.goto(request.url, { waitUntil: 'domcontentloaded' });
      else if (request.history === 'back') await target.goBack({ waitUntil: 'domcontentloaded' });
      else if (request.history === 'forward') await target.goForward({ waitUntil: 'domcontentloaded' });
      else await target.reload({ waitUntil: 'domcontentloaded' });
      break;
    case 'snapshot':
      if (!request.ref) await enableAccessibility(target);
      return { page: await state(target, { ref: request.ref }) };
    case 'click':
      acceptNextDialog = Boolean(request.acceptDialog);
      try {
        await act(target, request, (found) => found.click({ button: request.button, clickCount: request.double ? 2 : 1 }));
        // A dialog opens right after the click.
        await settle(target);
      } finally {
        acceptNextDialog = false;
      }
      break;
    case 'clickPoints':
      acceptNextDialog = Boolean(request.acceptDialog);
      try {
        for (const [x, y] of request.points) {
          await target.mouse.click(x, y, { button: request.button, clickCount: request.double ? 2 : 1 });
          await target.waitForTimeout(POINT_PAUSE_MS);
        }
        await settle(target);
      } finally {
        acceptNextDialog = false;
      }
      break;
    case 'drag':
      await target.mouse.move(request.from[0], request.from[1]);
      await target.mouse.down();
      await target.mouse.move(request.to[0], request.to[1], { steps: DRAG_STEPS });
      await target.mouse.up();
      break;
    case 'type': {
      await act(target, request, async (field) => {
        await field.fill(request.text);
        if (request.submit) await field.press('Enter');
      });
      break;
    }
    case 'press':
      await target.keyboard.press(request.key);
      break;
    case 'select':
      await act(target, request, (found) => found.selectOption(request.values));
      break;
    case 'screenshot': {
      const shot = await screenshot(target, request);
      return { page: await state(target, { withSnapshot: false }), ...shot };
    }
    case 'tabs':
      if (request.action === 'new') {
        current = await context.newPage();
        if (request.url) await current.goto(request.url, { waitUntil: 'domcontentloaded' });
      } else if (request.action === 'select' || request.action === 'close') {
        const pages = context.pages();
        const index = request.index ?? pages.indexOf(current);
        const chosen = pages[index];
        if (!chosen) throw new Error(`No hay una pestaña ${index}: hay ${pages.length}.`);
        if (request.action === 'select') {
          current = chosen;
          await chosen.bringToFront();
        } else {
          await chosen.close();
        }
      }
      target = await page();
      break;
    case 'wait':
      if (request.text) await target.getByText(request.text).first().waitFor({ state: 'visible', timeout: request.timeoutMs });
      else if (request.textGone) await target.getByText(request.textGone).first().waitFor({ state: 'hidden', timeout: request.timeoutMs });
      else await target.waitForTimeout(request.timeMs);
      break;
    default:
      throw new Error(`Operación desconocida: ${request.op}`);
  }
  await settle(target);
  return { page: await state(target) };
}

// One request at a time, in order.
let queue = Promise.resolve();
const input = readline.createInterface({ input: process.stdin });
input.on('line', (line) => {
  queue = queue.then(async () => {
    let request;
    try {
      request = JSON.parse(line);
    } catch {
      return;
    }
    try {
      send({ id: request.id, ok: true, ...(await run(request)) });
    } catch (error) {
      send({ id: request.id, ok: false, error: String(error?.message ?? error) });
    }
    if (closing) process.exit(0);
  });
});
input.on('close', () => {
  queue.then(async () => {
    closing = true;
    await context?.close().catch(() => {});
    process.exit(0);
  });
});
