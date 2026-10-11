# HD texture packs

A texture pack replaces the game's textures with your own, usually upscaled versions, without
touching your installation. Nothing is shipped with the game, and packs made from the game's
textures are for your own use: a pack starts from your own converted installation.

With no pack chosen, nothing changes: maps load exactly as before.

## Using a pack

1. Put the pack folder in `texture-packs` beside `skate3rust.exe` (the folder that also holds
   `data` and `mods`): `texture-packs/<pack name>/`. `SKATE3_TEXTURE_PACKS` points elsewhere.
2. In the menu, **Graphics → Texture pack**, choose it (Left/Right cycles Off and every pack).
3. World textures change the next time a map loads (MAPS → a map → Skate from default spawn).
   Character textures change at the next start.

The log says what it did: `TEXTURE_PACK <pack>: replaced 212 of 230 textures in DownTown`, plus
a line for each file it could not use and why.

In the browser build ([web-port/installation.md](../web-port/installation.md)), choose the
package folder on the page. Its `texture-packs` folder comes with it, and the same menu row
chooses the pack.

## Making a pack

1. Write a map's textures out as PNG files. On Windows, from the package folder:

   ```
   skate3rust.exe --dump-textures data\installations\<id>\maps\DownTown.skate "texture-packs\My Pack"
   ```

   This writes `texture-packs/My Pack/DownTown/<texture name>.png`, one file per texture.
   Repeat it for each map you want to cover.
2. Edit or upscale the PNGs: AI upscalers, hand painting, anything that writes PNG. Keep each
   texture's **shape**. A 256×128 texture can become 1024×512, but not 1024×1024. At most
   8192 pixels a side.
3. Delete the files you did not change. Unchanged files only cost loading time and memory.

### Layout

```
texture-packs/
  My Pack/
    DownTown/brick_wall_02.png     replaces brick_wall_02 in DownTown only
    brick_wall_02.png              replaces it in every map that has it
    assets/private/default_skater/textures/decoded/<file>.png
                                   replaces the installation's image at that path
```

- **World textures** match by name, ignoring case:
  - `<map>/<texture>.png` is looked for first;
  - then `<texture>.png`, for a texture shared across maps.

  Characters a Windows file name cannot hold (`<>:"/\|?*`) are written as `_` by
  `--dump-textures` and expected that way. Pixels are used as stored, exactly as the dump wrote
  them; the texture keeps the map's colour space.
- **Images loaded through the asset server** (character and customiser textures): a file under
  the pack's `assets/` replaces the installation's file at the same path under `assets/`. Only
  `.png`, `.jpg` and `.jpeg` are replaced; models and data never are.
- **Not covered:**
  - Environment cube maps (six faces) are left alone.
  - The sky, HUD and menu images are read outside both paths.

## How it works

`crates/skate-game/src/texture_pack.rs`.
- **World textures.** After a map is parsed (startup and map changes alike), each named texture
  with a matching file gets that file's pixels and size. Everything downstream (texture pages,
  mips, anisotropic filtering, both renderers) builds from the map as usual. Pages group textures
  by size, so replacements simply land in pages of their new size.
- **Asset images.** For the asset server, a chosen pack with an `assets` folder becomes a layer
  over the default source. With no pack, no layer is registered.
- **In the browser,** pack files are read from disk when the map that needs them is parsed: startup
  and map changes notice the files they missed, read them and start again.

## Memory and size

A texture at twice the width and height takes four times the memory, and every replaced texture
stays in memory while its map is loaded. Large packs can exhaust video memory on smaller GPUs, and
the browser build has a 4 GB ceiling for everything it holds. Packs that replace only the textures
that matter most (ground, walls, the skater) go a long way.
