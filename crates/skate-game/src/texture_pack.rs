//! HD texture packs: replacement textures from a folder the player fills
//! (docs/graphics/texture-packs.md). Nothing is shipped. A pack starts from the player's own map
//! textures, written out with `--dump-textures`, then upscaled or repainted.
//!
//! Packs live in `texture-packs/<pack>/` beside the game, like `mods/` (`SKATE3_TEXTURE_PACKS`
//! overrides; the browser build uses the chosen package folder's `texture-packs/`). One pack is
//! chosen in the graphics menu (`texture_pack` in `settings/graphics.json`):
//! - `<map>/<texture>.png` or `<texture>.png` replaces the world texture of that name in that map,
//!   or in every map, when a map loads. Same aspect ratio, any size up to 8192.
//! - `assets/<path>` replaces the installation's image at `assets/<path>` (character and
//!   customiser textures, which the asset server loads), from the next start.
//!
//! The default, no pack, changes nothing: maps load exactly as before.
use bevy::log::warn;
use skate_data::skate_map::{SkateMap, Texture};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Component, Path, PathBuf},
};

/// The largest replacement side: WebGPU's default `maxTextureDimension2D`, and what every
/// renderer this game runs on supports.
pub(crate) const MAX_SIDE: u32 = 8192;

/// Image types a pack can replace through the asset server.
const ASSET_IMAGES: [&str; 3] = ["png", "jpg", "jpeg"];

/// Where packs live: `SKATE3_TEXTURE_PACKS`, else `texture-packs` beside the executable.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn root() -> PathBuf {
    std::env::var_os("SKATE3_TEXTURE_PACKS").map(PathBuf::from).unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("texture-packs")))
            .unwrap_or_else(|| "texture-packs".into())
    })
}

/// In the browser: the chosen package folder's `texture-packs`, which the page mounts beside the
/// installation (`web/installation.js`).
#[cfg(target_arch = "wasm32")]
pub(crate) fn root() -> PathBuf {
    Path::new(skate_fs::ROOT).join("texture-packs")
}

/// The pack folder names, in order.
pub(crate) fn available() -> Vec<String> {
    let mut packs: Vec<String> = skate_fs::read_dir(root())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| valid_name(name))
        .collect();
    packs.sort_by_key(|name| name.to_lowercase());
    packs
}

/// A pack is one folder directly under the root.
pub(crate) fn valid_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!((parts.next(), parts.next()), (Some(Component::Normal(_)), None)) && !name.starts_with('.')
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct Saved {
    texture_pack: Option<String>,
}

/// The chosen pack's folder, if one is chosen and present.
pub(crate) fn selected(asset_root: &Path) -> Option<PathBuf> {
    let saved: Saved = skate_fs::read(crate::graphics_quality::settings_path(asset_root))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let name = saved.texture_pack.filter(|name| valid_name(name))?;
    let dir = root().join(&name);
    if skate_fs::is_dir(&dir) {
        Some(dir)
    } else {
        warn!("TEXTURE_PACK {name:?} is chosen but {} is not a folder", dir.display());
        None
    }
}

/// A texture name as a file name: what `--dump-textures` writes and a pack provides. Characters a
/// file name cannot hold on Windows (and path separators) become `_`.
pub(crate) fn file_name(texture: &str) -> String {
    let name: String = texture
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    // Windows also refuses names ending in a dot or space.
    let name = name.trim_end_matches(['.', ' ']);
    if name.is_empty() { "_".into() } else { name.into() }
}

/// A single-layer texture; environment cubes hold six faces and are left alone.
fn is_2d(texture: &Texture) -> bool {
    texture.rgba.len() as u64 == u64::from(texture.width) * u64::from(texture.height) * 4
}

/// Every file under `dir`, keyed by its `/`-separated path relative to `dir`, lowercased: packs
/// made on Windows match whatever case the texture names use.
fn index(dir: &Path) -> HashMap<String, PathBuf> {
    fn walk(dir: &Path, prefix: &str, out: &mut HashMap<String, PathBuf>) {
        for entry in skate_fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(name) = entry.file_name().into_string() else { continue };
            let key = if prefix.is_empty() { name.to_lowercase() } else { format!("{prefix}/{}", name.to_lowercase()) };
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                walk(&entry.path(), &key, out);
            } else {
                out.insert(key, entry.path());
            }
        }
    }
    let mut out = HashMap::new();
    walk(dir, "", &mut out);
    out
}

/// A replacement image as RGBA8, at most [`MAX_SIDE`] a side (checked before decoding).
pub(crate) fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let (width, height) = reader.into_dimensions().map_err(|e| e.to_string())?;
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!("{width}x{height}: each side must be 1 to {MAX_SIDE}"));
    }
    let rgba = image::load_from_memory(bytes).map_err(|e| e.to_string())?.into_rgba8().into_raw();
    Ok((width, height, rgba))
}

/// UVs are relative, so any size works that keeps the texture's shape.
fn same_aspect(width: u32, height: u32, texture: &Texture) -> bool {
    u64::from(width) * u64::from(texture.height) == u64::from(height) * u64::from(texture.width)
}

/// What applying a pack to one map did.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Applied {
    pub replaced: usize,
    pub textures: usize,
    /// Pack files that matched a texture but could not replace it, and why.
    pub skipped: Vec<String>,
}

/// Replaces `map`'s textures from the pack in `pack`: `<map stem>/<name>.png` first, then
/// `<name>.png`. Pixels stay as the pack stores them, raw like the map's own (`--dump-textures`
/// writes them the same way); the colour space stays the map's. Fails only in the browser, when
/// pack files were not read yet: startup and map changes read them and try again.
pub(crate) fn apply(map: &mut SkateMap, map_stem: &str, pack: &Path) -> Result<Applied, String> {
    let index = index(pack);
    let stem = map_stem.to_lowercase();
    // Each pack file once, with the textures it replaces (usually one).
    let mut users: BTreeMap<&PathBuf, Vec<usize>> = BTreeMap::new();
    for (i, texture) in map.textures.iter().enumerate() {
        if texture.name.is_empty() || !is_2d(texture) {
            continue;
        }
        let file = format!("{}.png", file_name(&texture.name)).to_lowercase();
        if let Some(path) = index.get(&format!("{stem}/{file}")).or_else(|| index.get(&file)) {
            users.entry(path).or_default().push(i);
        }
    }
    let mut applied = Applied { textures: map.textures.len(), ..Default::default() };
    let mut not_read = false;
    for (path, textures) in users {
        let (width, height, rgba) = match skate_fs::read(path) {
            // Read every file this map wants before failing, so one retry reads them all.
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                not_read = true;
                continue;
            }
            Err(error) => {
                applied.skipped.push(format!("{}: {error}", path.display()));
                continue;
            }
            Ok(bytes) => match decode(&bytes) {
                Ok(image) => image,
                Err(why) => {
                    applied.skipped.push(format!("{}: {why}", path.display()));
                    continue;
                }
            },
        };
        let (fits, misfits): (Vec<usize>, Vec<usize>) =
            textures.into_iter().partition(|&i| same_aspect(width, height, &map.textures[i]));
        for i in misfits {
            let texture = &map.textures[i];
            applied.skipped.push(format!(
                "{}: {width}x{height} does not keep the {}x{} shape of {:?}",
                path.display(),
                texture.width,
                texture.height,
                texture.name
            ));
        }
        // The pixels move into the last texture; any others sharing the file get copies.
        let mut rgba = Some(rgba);
        let last = fits.len().saturating_sub(1);
        for (n, i) in fits.into_iter().enumerate() {
            let pixels = if n == last { rgba.take().unwrap_or_default() } else { rgba.clone().unwrap_or_default() };
            let texture = &mut map.textures[i];
            (texture.width, texture.height, texture.rgba) = (width, height, pixels);
            applied.replaced += 1;
        }
    }
    if not_read {
        return Err(format!("texture pack {}: files not read in advance", pack.display()));
    }
    Ok(applied)
}

/// Loads a map with the chosen pack applied (`texture-packs`, graphics menu). Without one, this is
/// `SkateMap::load`.
pub(crate) fn load_map(path: &Path, asset_root: &Path) -> Result<SkateMap, String> {
    let mut map = SkateMap::load(path)?;
    if let Some(pack) = selected(asset_root) {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let applied = apply(&mut map, &stem, &pack)?;
        for why in applied.skipped.iter().take(20) {
            report(&format!("TEXTURE_PACK skipped {why}"));
        }
        if applied.skipped.len() > 20 {
            report(&format!("TEXTURE_PACK skipped {} more", applied.skipped.len() - 20));
        }
        report(&format!(
            "TEXTURE_PACK {}: replaced {} of {} textures in {stem}",
            pack.file_name().unwrap_or_default().to_string_lossy(),
            applied.replaced,
            applied.textures
        ));
    }
    Ok(map)
}

/// The log natively; in the browser, the console directly, since the start map loads before Bevy's
/// logger exists there.
fn report(text: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    bevy::log::info!("{text}");
    #[cfg(target_arch = "wasm32")]
    crate::web::console(text);
}

/// The chosen pack's `assets` folder, when it has one: images there replace the installation's at
/// the same path (`register_assets`).
pub(crate) fn asset_overlay(asset_root: &Path) -> Option<PathBuf> {
    Some(selected(asset_root)?.join("assets")).filter(|dir| skate_fs::is_dir(dir))
}

/// Whether the asset server should look in the pack first: images only, so a pack replaces
/// textures, never models or data.
pub(crate) fn is_asset_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| ASSET_IMAGES.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// The asset server's default source with a pack over it: an image the pack has at the same path
/// wins, everything else comes from `base`.
pub(crate) struct Overlay {
    pub pack: Box<dyn bevy::asset::io::ErasedAssetReader>,
    pub base: Box<dyn bevy::asset::io::ErasedAssetReader>,
}

impl bevy::asset::io::AssetReader for Overlay {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl bevy::asset::io::Reader + 'a, bevy::asset::io::AssetReaderError> {
        if is_asset_image(path) {
            match self.pack.read(path).await {
                Err(bevy::asset::io::AssetReaderError::NotFound(_)) => {}
                found => return found,
            }
        }
        self.base.read(path).await
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl bevy::asset::io::Reader + 'a, bevy::asset::io::AssetReaderError> {
        self.base.read_meta(path).await
    }

    async fn read_directory<'a>(&'a self, path: &'a Path) -> Result<Box<bevy::asset::io::PathStream>, bevy::asset::io::AssetReaderError> {
        self.base.read_directory(path).await
    }

    async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, bevy::asset::io::AssetReaderError> {
        self.base.is_directory(path).await
    }
}

/// Before `DefaultPlugins`: with a chosen pack that has `assets/`, the default source reads it
/// first (natively; the browser's installation reader does the same in `web`). Without one,
/// nothing is registered and the asset server is Bevy's own.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn register_assets(app: &mut bevy::app::App, asset_root: &Path) {
    use bevy::asset::{AssetApp, io::{AssetSourceBuilder, AssetSourceId, file::FileAssetReader}};
    let Some(pack) = asset_overlay(asset_root) else { return };
    report(&format!("TEXTURE_PACK images from {}", pack.display()));
    let base = asset_root.to_owned();
    app.register_asset_source(
        AssetSourceId::Default,
        AssetSourceBuilder::new(move || {
            Box::new(Overlay { pack: Box::new(FileAssetReader::new(pack.clone())), base: Box::new(FileAssetReader::new(base.clone())) })
        }),
    );
}

/// `--dump-textures <map.skate> <folder>`: writes the map's textures as `<folder>/<map>/<name>.png`,
/// the layout a pack uses, so a pack starts from the player's own textures. Returns how many it
/// wrote. Pixels are written raw, exactly as the map stores them.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn dump(map_path: &Path, out: &Path) -> Result<usize, String> {
    let map = SkateMap::load(map_path)?;
    let stem = map_path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    dump_map(&map, &out.join(&stem))
}

#[cfg(not(target_arch = "wasm32"))]
fn dump_map(map: &SkateMap, dir: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut written: HashMap<String, &Texture> = HashMap::new();
    for texture in &map.textures {
        if texture.name.is_empty() || !is_2d(texture) {
            continue;
        }
        let file = format!("{}.png", file_name(&texture.name));
        match written.get(&file.to_lowercase()) {
            Some(first) if first.rgba != texture.rgba => {
                warn!("--dump-textures: {:?} shares {file} with a different texture; the first one is written", texture.name);
                continue;
            }
            Some(_) => continue,
            None => {}
        }
        let path = dir.join(&file);
        image::save_buffer(&path, &texture.rgba, texture.width, texture.height, image::ColorType::Rgba8)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        written.insert(file.to_lowercase(), texture);
    }
    Ok(written.len())
}

#[cfg(test)]
#[path = "tests/texture_pack.rs"]
mod tests;
