// Executes the actual inline loader/fatal script in a minimal DOM fixture. It does not
// certify browser rendering, downloads, WASM startup or a real engine panic.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { runInNewContext } from 'node:vm';
import test from 'node:test';

const htmlPath = process.env.FILMCRAFT_INDEX_HTML ?? fileURLToPath(new URL('../web/index.html', import.meta.url));
const html = readFileSync(htmlPath, 'utf8');
const source = html.match(/<script id="lts-editor-presentation">([\s\S]*?)<\/script>/)?.[1];
assert.ok(source, 'test runs the actual presentation/fatal script');

function fixture(search) {
  const nodes = new Map();
  function node(id, textContent = '') {
    const value = {
      textContent, hidden: false, disabled: false, onclick: null, removed: false,
      classes: new Set(), remove() { this.removed = true; },
      querySelector(selector) { assert.equal(selector, 'img'); return nodes.get('loading-img'); },
    };
    value.classList = { add: (name) => value.classes.add(name) };
    nodes.set(id, value);
    return value;
  }
  node('loading'); node('loading-img'); node('favicon'); node('fatal'); node('fatal-msg'); node('fatal-reload');
  for (const id of ['status', 'fatal-title', 'fatal-preservation']) {
    const match = html.match(new RegExp(`<[^>]+id="${id}"[^>]*>([^<]*)<`));
    assert.ok(match, `actual initial ${id} copy exists`);
    node(id, match[1]);
  }
  const document = {
    title: html.match(/<title>([^<]*)<\/title>/)[1],
    getElementById(id) { assert.ok(nodes.has(id), id); return nodes.get(id); },
    querySelector(selector) { assert.equal(selector, 'link[rel="icon"]'); return nodes.get('favicon'); },
  };
  const events = new Map();
  let reloads = 0;
  const sandbox = {
    document, window: {}, URLSearchParams, WebAssembly,
    location: { search, reload() { reloads += 1; } },
    console: { error() {} }, addEventListener(type, callback) { events.set(type, callback); },
  };
  runInNewContext(source, sandbox, { filename: htmlPath, timeout: 1000 });
  return { nodes, document, sandbox, events, reloads: () => reloads };
}

for (const search of ['?embedded', '?embedded=', '?embedded=1', '?embedded=false', '?%65mbedded']) {
  test(`embedded presence flag keeps truthful volatile failure copy: ${search}`, () => {
    const f = fixture(search);
    assert.equal(f.document.title, 'LTS Video editor');
    assert.equal(f.nodes.get('status').textContent, 'Loading LTS Video editor…');
    assert.equal(f.nodes.get('loading-img').removed, true);
    assert.equal(f.nodes.get('favicon').removed, true);
    assert.equal(f.nodes.get('fatal-title').textContent, 'LTS Video editor stopped working');
    assert.match(f.nodes.get('fatal-preservation').textContent, /not saved to LTS/);
    assert.match(f.nodes.get('fatal-preservation').textContent, /Reloading would discard/);
    const reload = f.nodes.get('fatal-reload');
    assert.equal(reload.hidden, true);
    assert.equal(reload.disabled, true);
    assert.equal(reload.onclick, null);
    const failure = '<img src=x onerror=alert(1)> fixture failure';
    f.sandbox.window.filmcraftFatal(failure);
    assert.equal(f.sandbox.window.filmcraftLoad.fatal, failure);
    assert.equal(f.nodes.get('fatal-msg').textContent, failure);
    assert.ok(f.nodes.get('fatal').classes.has('on'));
    f.sandbox.window.filmcraftFatal('later failure');
    assert.equal(f.nodes.get('fatal-msg').textContent, failure, 'first failure remains visible');
    assert.equal(f.reloads(), 0);
  });
}

for (const search of ['', '?embeddedness=1', '?mode=embedded']) {
  test(`standalone loader and explicit reload remain unchanged: ${search}`, () => {
    const f = fixture(search);
    assert.equal(f.document.title, 'FilmCraft');
    assert.equal(f.nodes.get('status').textContent, 'Loading FilmCraft…');
    assert.equal(f.nodes.get('loading-img').removed, false);
    assert.equal(f.nodes.get('favicon').removed, false);
    assert.equal(f.nodes.get('fatal-title').textContent, 'FilmCraft stopped working');
    assert.equal(f.nodes.get('fatal-preservation').textContent, 'Unsaved changes are kept in the browser every few seconds and come back when you reload.');
    const reload = f.nodes.get('fatal-reload');
    assert.equal(reload.hidden, false);
    assert.equal(reload.disabled, false);
    assert.equal(typeof reload.onclick, 'function');
    assert.equal(f.reloads(), 0);
    reload.onclick();
    assert.equal(f.reloads(), 1);
  });
}

test('host injection markers stay unique', () => {
  // The LTS host injects its bridge with html.replace("</body>", …) and this test runs exactly
  // one presentation script: both markers must stay single.
  assert.equal(html.split('</body>').length, 2, 'exactly one literal </body>');
  assert.equal(html.split('id="lts-editor-presentation"').length, 2, 'exactly one presentation script');
});

test('embedded copy uses the LTS Video editor name everywhere, including start-up failure', () => {
  assert.equal(html.includes('LTS video editor'), false, 'no lowercase "video" variant remains');
  const startup = html.match(/<script type="module">([\s\S]*?)<\/script>/)?.[1];
  assert.ok(startup, 'module start-up script exists');
  assert.match(startup, /params\.has\("embedded"\) \? "LTS Video editor" : "FilmCraft"/);
});

test('caught runtime error retains original diagnostic and never reloads embedded session', () => {
  const f = fixture('?embedded');
  f.sandbox.window.filmcraftLoad.readyMs = 12;
  f.sandbox.console.error('panicked at', 'original diagnostic\n\nStack:omitted stack');
  f.events.get('error')({ error: new WebAssembly.RuntimeError('unreachable') });
  assert.equal(f.nodes.get('fatal-msg').textContent, 'panicked at original diagnostic');
  assert.equal(f.sandbox.window.filmcraftLoad.fatal, 'panicked at original diagnostic\n\nStack:omitted stack');
  assert.equal(f.reloads(), 0);
});
