// The player's own converted installation, read from the folder they choose. Nothing is uploaded:
// the engine asks for files through `globalThis.skateInstallation` (crates/skate-fs/src/web.rs)
// and they are read from this computer's disk. Settings the engine writes are kept in this
// browser's IndexedDB, per installation, so they survive a reload.

const MARKER = 'assets/private/game.json';
const PARALLEL_READS = 16;

/** Every file under a chosen folder, as `[relative path, File]`. */
async function walk(handle, prefix, out, progress) {
  const files = [];
  for await (const entry of handle.values()) {
    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.kind === 'directory') await walk(entry, path, out, progress);
    else files.push([path, entry]);
  }
  // A File is a reference to the file on disk; nothing is read yet.
  for (const [path, file] of await Promise.all(files.map(async ([path, entry]) => [path, await entry.getFile()]))) {
    out.push([path, file]);
  }
  if (files.length) progress(`Listing files… ${out.length.toLocaleString()}`);
  return out;
}

/** The directory holding `assets/`, `maps/` and `settings/`, from any folder above it. */
function locate(paths, markerJson) {
  const roots = paths.filter((p) => p === MARKER || p.endsWith(`/${MARKER}`)).map((p) => p.slice(0, -MARKER.length).replace(/\/$/, ''));
  if (roots.length === 0) {
    if (paths.some((p) => p === 'private/game.json' || p.endsWith('/private/game.json'))) {
      throw new Error('That is the assets folder. Choose the folder above it, which holds assets, maps and settings.');
    }
    throw new Error('No converted installation in that folder. Choose your Skate 3 Rust Engine package folder, its data folder, or data/installations/<id>. Convert your copy with the native setup first.');
  }
  if (roots.length === 1) return roots[0];
  // Several installations under data/installations: the marker names the current one.
  if (markerJson) {
    const named = roots.find((root) => root === markerJson.root || root.endsWith(`/${markerJson.directory}`) || root === markerJson.directory);
    if (named !== undefined) return named;
  }
  throw new Error(`That folder holds ${roots.length} installations and no data/installation.json says which is current. Choose one data/installations/<id> folder.`);
}

/** This browser's store for settings and the remembered folder; `null` where there is none. */
function openSaved() {
  return new Promise((resolve) => {
    try {
      const request = indexedDB.open('skate3-web', 1);
      request.onupgradeneeded = () => {
        request.result.createObjectStore('saved');
        request.result.createObjectStore('folders');
      };
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => resolve(null);
    } catch {
      resolve(null);
    }
  });
}

function stored(db, store, mode, action) {
  if (!db) return Promise.resolve(undefined);
  return new Promise((resolve) => {
    try {
      const request = action(db.transaction(store, mode).objectStore(store));
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => resolve(undefined);
    } catch {
      resolve(undefined);
    }
  });
}

/** A folder handle chosen on an earlier visit (File System Access API only). */
export async function rememberedFolder() {
  const db = await openSaved();
  return stored(db, 'folders', 'readonly', (folders) => folders.get('last'));
}

/** Asks the browser for the folder: the File System Access API where there is one, else a directory input. */
export async function chooseFolder(input, progress) {
  if ('showDirectoryPicker' in window) {
    const handle = await window.showDirectoryPicker({ id: 'skate3-installation', mode: 'read' });
    const db = await openSaved();
    await stored(db, 'folders', 'readwrite', (folders) => folders.put(handle, 'last'));
    return fromHandle(handle, progress);
  }
  const files = await new Promise((resolve, reject) => {
    input.onchange = () => resolve([...input.files]);
    input.oncancel = () => reject(new DOMException('No folder chosen', 'AbortError'));
    input.click();
  });
  return fromFiles(files);
}

export async function fromHandle(handle, progress) {
  if ((await handle.queryPermission({ mode: 'read' })) !== 'granted' && (await handle.requestPermission({ mode: 'read' })) !== 'granted') {
    throw new Error('The browser did not allow reading that folder.');
  }
  progress('Listing files…');
  return { name: handle.name, entries: await walk(handle, '', [], progress) };
}

/** Files from `<input webkitdirectory>`: paths start with the chosen folder's own name. */
export function fromFiles(files) {
  return {
    name: files[0]?.webkitRelativePath.split('/')[0] ?? '',
    entries: files.map((file) => [file.webkitRelativePath.split('/').slice(1).join('/'), file]),
  };
}

/**
 * Finds the installation in the chosen folder (`{ name, entries }`) and installs
 * `globalThis.skateInstallation` for the engine. `progress(text)` reports reads in flight.
 */
export async function mount({ name, entries }, progress) {
  if (entries.length === 0) throw new Error('That folder is empty.');
  const byPath = new Map(entries);
  let markerJson = null;
  const markerPath = [...byPath.keys()].find((p) => p === 'installation.json' || p.endsWith('/installation.json'));
  if (markerPath) {
    try {
      markerJson = JSON.parse(await byPath.get(markerPath).text());
      markerJson.root = [markerPath.slice(0, -'installation.json'.length), markerJson.directory].join('');
    } catch {
      markerJson = null;
    }
  }
  const root = locate([...byPath.keys()], markerJson);
  const prefix = root ? `${root}/` : '';
  const files = new Map();
  for (const [path, entry] of byPath) {
    if (path.startsWith(prefix)) files.set(path.slice(prefix.length), entry);
  }
  // An installation is named by its folder (data/installations/<id>), or by the folder chosen.
  const id = root.split('/').pop() || name || 'installation';
  // Texture packs live beside the game, in the package folder's texture-packs
  // (docs/graphics/texture-packs.md): mounted beside the installation when the package was chosen.
  const tail = `data/installations/${id}`;
  if (root === tail || root.endsWith(`/${tail}`)) {
    const packs = `${root.slice(0, root.length - tail.length)}texture-packs/`;
    for (const [path, entry] of byPath) {
      if (path.startsWith(packs)) files.set(`texture-packs/${path.slice(packs.length)}`, entry);
    }
  }

  const sizes = new Map([...files].map(([path, file]) => [path, file.size]));
  const cache = new Map();
  const db = await openSaved();
  const savedKeys = (await stored(db, 'saved', 'readonly', (saved) => saved.getAllKeys())) ?? [];
  for (const key of savedKeys.filter((key) => key.startsWith(`${id}/`))) {
    const bytes = await stored(db, 'saved', 'readonly', (saved) => saved.get(key));
    if (bytes instanceof Uint8Array) {
      const path = key.slice(id.length + 1);
      cache.set(path, bytes);
      sizes.set(path, bytes.length);
    }
  }

  let readBytes = 0;
  const read = async (path) => {
    const file = files.get(path);
    if (!file) throw new Error(`${path}: not in the installation`);
    const bytes = new Uint8Array(await file.arrayBuffer());
    readBytes += bytes.length;
    return bytes;
  };

  globalThis.skateInstallation = {
    id,
    files: () => [...sizes].map(([path, size]) => [path, size]),
    read: (path) => cache.get(path),
    fetch: (path) => (cache.has(path) ? Promise.resolve(cache.get(path)) : read(path)),
    async prefetch(paths) {
      const queue = paths.filter((path) => !cache.has(path));
      const total = queue.reduce((sum, path) => sum + (sizes.get(path) ?? 0), 0);
      const start = readBytes;
      const report = () => progress(`Reading your installation… ${mb(readBytes - start)} of ${mb(total)}`);
      report();
      const timer = setInterval(report, 250);
      try {
        await Promise.all(Array.from({ length: PARALLEL_READS }, async () => {
          for (let path; (path = queue.shift()) !== undefined;) cache.set(path, await read(path));
        }));
      } finally {
        clearInterval(timer);
      }
    },
    release: (path) => { if (!savedKeys.includes(`${id}/${path}`)) cache.delete(path); },
    saved(path, bytes) {
      const copy = bytes.slice();
      cache.set(path, copy);
      sizes.set(path, copy.length);
      if (!savedKeys.includes(`${id}/${path}`)) savedKeys.push(`${id}/${path}`);
      stored(db, 'saved', 'readwrite', (saved) => saved.put(copy, `${id}/${path}`));
    },
    removed(path) {
      const key = `${id}/${path}`;
      if (!savedKeys.includes(key)) return;
      savedKeys.splice(savedKeys.indexOf(key), 1);
      cache.delete(path);
      if (!files.has(path)) sizes.delete(path);
      stored(db, 'saved', 'readwrite', (saved) => saved.delete(key));
    },
  };
  return { id, files: files.size, packs: [...files.keys()].some((path) => path.startsWith('texture-packs/')) };
}

function mb(bytes) {
  return `${(bytes / 1048576).toFixed(bytes < 10485760 ? 1 : 0)} MB`;
}
