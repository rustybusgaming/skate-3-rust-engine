use super::*;
use bevy::asset::io::{AssetReader, memory::{Dir, MemoryAssetReader}};
use skate_data::skate_map::Texture;

fn texture(name: &str, width: u32, height: u32, value: u8) -> Texture {
    Texture { name: name.into(), width, height, color_space: 1, rgba: vec![value; (width * height * 4) as usize] }
}

/// The repository's procedural demo map, with synthetic textures.
fn map(textures: Vec<Texture>) -> SkateMap {
    let mut map = SkateMap::parse(include_bytes!("../../../../maps/format-demo.skate")).unwrap();
    map.textures = textures;
    map
}

fn png(width: u32, height: u32, value: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_raw(width, height, vec![value; (width * height * 4) as usize])
        .unwrap()
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("skate-texture-pack-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn put(dir: &Path, path: &str, bytes: &[u8]) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn texture_names_become_windows_safe_file_names() {
    assert_eq!(file_name("dt_brick_01"), "dt_brick_01");
    assert_eq!(file_name("env/sky:night*"), "env_sky_night_");
    assert_eq!(file_name("trailing. "), "trailing");
    assert_eq!(file_name(""), "_");
    assert!(valid_name("My Pack"));
    for bad in ["", ".", "..", "a/b", ".hidden", "/abs"] {
        assert!(!valid_name(bad), "{bad:?}");
    }
}

#[test]
fn packs_replace_textures_by_name_map_files_first() {
    let pack = folder("apply");
    put(&pack, "Brick.PNG", &png(4, 4, 10));
    put(&pack, "demo/brick.png", &png(8, 8, 20));
    put(&pack, "grass.png", &png(8, 4, 30));
    put(&pack, "tall.png", &png(4, 8, 40));
    put(&pack, "huge.png", &png(MAX_SIDE + 1, 1, 50));
    put(&pack, "broken.png", b"not a png");
    let cube = Texture { rgba: vec![0; 2 * 2 * 4 * 6], ..texture("sky", 2, 2, 0) };
    let mut map = map(vec![
        texture("BRICK", 2, 2, 1),
        texture("grass", 4, 2, 2),
        texture("tall", 2, 2, 3),
        texture("huge", 4, 4, 4),
        texture("broken", 2, 2, 5),
        texture("", 2, 2, 6),
        cube,
        texture("untouched", 2, 2, 7),
    ]);
    let applied = apply(&mut map, "Demo", &pack).unwrap();
    assert_eq!((applied.replaced, applied.textures), (2, 8));
    // The map's own folder wins over the pack root, whatever the case.
    assert_eq!((map.textures[0].width, map.textures[0].rgba[0]), (8, 20));
    // Same 2:1 shape, twice the size; the colour space stays the map's.
    assert_eq!((map.textures[1].width, map.textures[1].height, map.textures[1].rgba[0], map.textures[1].color_space), (8, 4, 30, 1));
    assert_eq!(applied.skipped.len(), 3, "{:?}", applied.skipped);
    assert!(applied.skipped.iter().any(|s| s.contains("tall.png") && s.contains("shape")));
    assert!(applied.skipped.iter().any(|s| s.contains("huge.png") && s.contains("8192")));
    assert!(applied.skipped.iter().any(|s| s.contains("broken.png")));
    for (index, value) in [(2, 3), (3, 4), (4, 5), (5, 6), (7, 7)] {
        assert_eq!((map.textures[index].width, map.textures[index].rgba[0]), (if index == 3 { 4 } else { 2 }, value));
    }
    assert_eq!(map.textures[6].rgba.len(), 2 * 2 * 4 * 6, "environment cubes are left alone");
    let _ = std::fs::remove_dir_all(pack);
}

#[test]
fn a_dumped_map_is_a_pack_that_changes_nothing() {
    let out = folder("dump");
    let original = map(vec![texture("a", 2, 2, 9), texture("b/c", 4, 2, 8), texture("a", 2, 2, 9)]);
    assert_eq!(dump_map(&original, &out.join("Demo")).unwrap(), 2);
    assert!(out.join("Demo/a.png").is_file() && out.join("Demo/b_c.png").is_file());
    let mut map = map(vec![texture("a", 2, 2, 9), texture("b/c", 4, 2, 8), texture("a", 2, 2, 9)]);
    let applied = apply(&mut map, "demo", &out).unwrap();
    assert_eq!(applied.replaced, 3);
    assert_eq!(map.textures, original.textures);
    let _ = std::fs::remove_dir_all(out);
}

#[test]
fn the_asset_overlay_replaces_images_only() {
    let (pack, base) = (Dir::default(), Dir::default());
    pack.insert_asset_text(Path::new("textures/skin.png"), "pack image");
    pack.insert_asset_text(Path::new("skater.glb"), "pack model");
    base.insert_asset_text(Path::new("textures/skin.png"), "base image");
    base.insert_asset_text(Path::new("textures/other.png"), "base other");
    base.insert_asset_text(Path::new("skater.glb"), "base model");
    let overlay = Overlay { pack: Box::new(MemoryAssetReader { root: pack }), base: Box::new(MemoryAssetReader { root: base }) };
    let read = |path: &str| {
        bevy::tasks::block_on(async {
            let mut reader = overlay.read(Path::new(path)).await.unwrap();
            let mut bytes = Vec::new();
            bevy::asset::io::Reader::read_to_end(&mut reader, &mut bytes).await.unwrap();
            String::from_utf8(bytes).unwrap()
        })
    };
    assert_eq!(read("textures/skin.png"), "pack image");
    assert_eq!(read("textures/other.png"), "base other", "an image the pack lacks comes from the installation");
    assert_eq!(read("skater.glb"), "base model", "a pack replaces images, never models");
}
