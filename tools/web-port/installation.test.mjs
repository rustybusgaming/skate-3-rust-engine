// The loader page's installation finder and store (web/installation.js), on plain Files:
//   node --test tools/web-port/installation.test.mjs
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { fromFiles, mount } from '../../web/installation.js';

const ID = '0123456789abcdef0123456789abcdef';
const OTHER = 'fedcba9876543210fedcba9876543210';

/** Files as `<input webkitdirectory>` lists them, under a chosen folder named `name`. */
function folder(name, files) {
  return fromFiles(Object.entries(files).map(([path, text]) => {
    const file = new File([text], path.split('/').pop());
    Object.defineProperty(file, 'webkitRelativePath', { value: `${name}/${path}` });
    return file;
  }));
}

const installation = (prefix) => ({
  [`${prefix}assets/private/game.json`]: '{}',
  [`${prefix}settings/default-map.json`]: '"maps/a.skate"',
  [`${prefix}maps/a.skate`]: 'map',
});

test('finds the installation from the package folder, its data folder or itself', async () => {
  for (const [name, prefix] of [['package', `data/installations/${ID}/`], ['data', `installations/${ID}/`], [ID, '']]) {
    // Files beside the installation are not part of it.
    const beside = prefix ? { 'unrelated.txt': 'x' } : {};
    const found = await mount(folder(name, { ...installation(prefix), ...beside }), () => {});
    assert.equal(found.id, ID, name);
    assert.equal(found.files, 3, name);
    const files = new Map(globalThis.skateInstallation.files());
    assert.deepEqual([...files.keys()].sort(), ['assets/private/game.json', 'maps/a.skate', 'settings/default-map.json']);
    assert.equal(files.get('maps/a.skate'), 3);
  }
});

test('texture packs beside the package are mounted with it', async () => {
  const files = {
    ...installation(`data/installations/${ID}/`),
    'texture-packs/HD/DownTown/brick.png': 'png',
    'texture-packs/HD/assets/private/skin.png': 'png',
  };
  const found = await mount(folder('package', files), () => {});
  assert.equal(found.packs, true);
  const mounted = new Map(globalThis.skateInstallation.files());
  assert.equal(mounted.get('texture-packs/HD/DownTown/brick.png'), 3);
  assert.ok(mounted.has('texture-packs/HD/assets/private/skin.png'));
  // From the data folder the packs are out of reach.
  const fromData = Object.fromEntries(Object.entries(files).map(([path, text]) => [path.replace(/^data\//, ''), text]));
  assert.equal((await mount(folder('data', fromData), () => {})).packs, false);
});

test('the marker picks between installations', async () => {
  const files = {
    'data/installation.json': JSON.stringify({ version: 1, directory: `installations/${OTHER}` }),
    ...installation(`data/installations/${ID}/`),
    ...installation(`data/installations/${OTHER}/`),
  };
  assert.equal((await mount(folder('package', files), () => {})).id, OTHER);
  delete files['data/installation.json'];
  await assert.rejects(mount(folder('package', files), () => {}), /2 installations/);
});

test('refuses folders without one, and says so for the assets folder', async () => {
  await assert.rejects(mount(folder('x', { 'a.txt': '' }), () => {}), /No converted installation/);
  await assert.rejects(mount(folder('assets', { 'private/game.json': '{}' }), () => {}), /assets folder/);
});

test('reads ahead, serves reads, keeps writes and forgets removed ones', async () => {
  await mount(folder('package', installation(`data/installations/${ID}/`)), () => {});
  const page = globalThis.skateInstallation;
  assert.equal(page.read('maps/a.skate'), undefined, 'nothing is read before it is asked for');
  await page.prefetch(['maps/a.skate', 'settings/default-map.json']);
  assert.equal(new TextDecoder().decode(page.read('maps/a.skate')), 'map');
  assert.equal(new TextDecoder().decode(await page.fetch('assets/private/game.json')), '{}');
  assert.equal(page.read('assets/private/game.json'), undefined, 'fetch does not hold the file');
  page.release('maps/a.skate');
  assert.equal(page.read('maps/a.skate'), undefined);

  page.saved('settings/graphics.json.tmp', new TextEncoder().encode('{"a":1}'));
  page.saved('settings/graphics.json', new TextEncoder().encode('{"a":1}'));
  page.removed('settings/graphics.json.tmp');
  const files = new Map(page.files());
  assert.equal(files.get('settings/graphics.json'), 7);
  assert.equal(files.has('settings/graphics.json.tmp'), false);
  page.release('settings/graphics.json');
  assert.equal(new TextDecoder().decode(page.read('settings/graphics.json')), '{"a":1}', 'written files stay');
});
