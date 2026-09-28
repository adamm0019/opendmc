//! End-to-end: a fake install made of synthetic files goes through every
//! subcommand exactly as a real install would.

use dmc_formats::Endian;
use dmc_formats::bdp::BundleBuilder;
use dmc_formats::bytes::Writer;
use dmc_formats::geometry::{self, NewMesh, STRIP_BREAK};
use dmc_formats::texture;
use std::fs;
use std::path::Path;
use std::process::Command;

fn model(e: Endian) -> Vec<u8> {
    let mesh = NewMesh {
        tex_index: 0,
        positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]],
        normals: vec![[0., 0., 1.]; 4],
        uvs: vec![[0., 0.]; 4],
        joints: vec![[0, 0, 0]; 4],
        weight_words: vec![STRIP_BREAK, STRIP_BREAK, 0, 0],
    };
    let geo = geometry::build(e, geometry::Variant::Ps3, 1, &[vec![mesh]], None);
    let px = [0x00, 0xF8, 0, 0, 0, 0, 0, 0];
    let tex = texture::build(
        texture::Kind::T32,
        e,
        &[texture::NewImage {
            format_code: 6,
            width: 4,
            height: 4,
            pixels: &px,
        }],
    );
    let mut w = Writer::new(e);
    let s1 = 16 + geo.len().next_multiple_of(16);
    w.u32(16)
        .u32(s1 as u32)
        .u32(0)
        .u32(0)
        .bytes(&geo)
        .pad_to(16, 0)
        .bytes(&tex);
    w.finish()
}

fn dmc(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_dmc"))
        .args(args)
        .output()
        .expect("run dmc");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "dmc {args:?} failed:\n{text}");
    text
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// A PC-layout model: counted section table, widened geometry, and a texture
/// container with the fixed magic bytes in front of little-endian fields.
fn pc_model() -> Vec<u8> {
    let e = Endian::Little;
    let mesh = NewMesh {
        tex_index: 0,
        positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]],
        normals: vec![[0., 0., 1.]; 4],
        uvs: vec![[0., 0.]; 4],
        joints: vec![[0, 0, 0]; 4],
        weight_words: vec![STRIP_BREAK, STRIP_BREAK, 0, 0],
    };
    let geo = geometry::build(e, geometry::Variant::Pc64, 1, &[vec![mesh]], None);
    let px = [0x00, 0xF8, 0, 0, 0, 0, 0, 0];
    let tex = texture::build(
        texture::Kind::T32,
        e,
        &[texture::NewImage {
            format_code: 6,
            width: 4,
            height: 4,
            pixels: &px,
        }],
    );
    let mut w = Writer::new(e);
    let s1 = 16 + geo.len().next_multiple_of(16);
    w.u32(2)
        .u32(16)
        .u32(s1 as u32)
        .u32(0)
        .bytes(&geo)
        .pad_to(16, 0)
        .bytes(&tex);
    w.finish()
}

fn write_nbz(path: &Path, files: &[(&str, Vec<u8>)]) {
    use std::io::Write;
    let mut z = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in files {
        z.start_file(*name, opts).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap();
}

#[test]
fn pc_archive_pipeline() {
    let root = std::env::temp_dir().join(format!("opendmc-smoke-pc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let install = root.join("install/data/dmc1");
    fs::create_dir_all(&install).unwrap();
    let nbz = install.join("dmc1-0.nbz");
    write_nbz(
        &nbz,
        &[
            ("Pld/pl00.pld", pc_model()),
            ("Emd/em00.emd", pc_model()),
            ("Etc/mystery.dat", vec![7; 64]),
        ],
    );

    let reports = root.join("reports");
    dmc(&["inventory", s(&root.join("install")), "--out", s(&reports)]);
    let md = fs::read_to_string(reports.join("inventory.md")).unwrap();
    assert!(
        md.contains("ZIP archives (`.nbz`): **1** holding **3** entries"),
        "{md}"
    );
    assert!(md.contains("Files parsed as DMC1 models: **2**"), "{md}");
    assert!(md.contains("models: Little ×2"), "{md}");
    assert!(md.contains("Counted ×2"), "{md}");
    assert!(md.contains("dmc1-0.nbz::Etc/mystery.dat"), "{md}");

    let spec = format!("{}::Pld/pl00.pld", nbz.display());
    let info = dmc(&["info", &spec]);
    assert!(info.contains("model: Little Counted"), "{info}");
    assert!(
        info.contains("geometry: 1 objects, 1 meshes, 4 vertices"),
        "{info}"
    );
    assert!(info.contains("T32 Little, 1 images"), "{info}");

    let export = dmc(&["export", s(&nbz), s(&root.join("export"))]);
    assert!(
        export.contains("2 converted (0 flagged for review), 0 failed"),
        "{export}"
    );
    gltf::import(root.join("export/Pld/pl00.glb")).expect("valid glTF");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn full_pipeline_on_a_synthetic_install() {
    let root = std::env::temp_dir().join(format!("opendmc-smoke-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let install = root.join("install");
    fs::create_dir_all(install.join("data/dmc1")).unwrap();

    let bundle = BundleBuilder::new(Endian::Little, "DMC1")
        .file("data/pld/pl00.pld", model(Endian::Little))
        .file("data/emd/em00.emd", model(Endian::Little))
        .file("data/snd/se.bin", vec![0x42; 300])
        .build();
    fs::write(install.join("data/dmc1/DMC1.BDP"), bundle).unwrap();
    fs::write(install.join("data/dmc1/loose.pws"), model(Endian::Big)).unwrap();
    fs::write(install.join("data/dmc1/music.ogg"), b"OggS\0\0\0\0").unwrap();

    let reports = root.join("reports");
    dmc(&["inventory", s(&install), "--out", s(&reports)]);
    let md = fs::read_to_string(reports.join("inventory.md")).unwrap();
    assert!(md.contains("Pipeworks bundles: **1**"), "{md}");
    assert!(md.contains("Files parsed as DMC1 models: **3**"), "{md}");
    assert!(
        md.contains("`data/snd/se.bin`"),
        "unknown entries are listed:\n{md}"
    );
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(reports.join("inventory.json")).unwrap()).unwrap();
    assert_eq!(json["files"].as_array().unwrap().len(), 3);

    let listing = dmc(&[
        "bundle-list",
        s(&install.join("data/dmc1/DMC1.BDP")),
        "--pattern",
        "*.pld",
    ]);
    assert!(listing.contains("data/pld/pl00.pld") && !listing.contains("em00"));

    let extracted = root.join("extract");
    dmc(&[
        "bundle-extract",
        s(&install.join("data/dmc1/DMC1.BDP")),
        s(&extracted),
    ]);
    assert!(extracted.join("data/emd/em00.emd").exists());

    let info = dmc(&["info", s(&extracted.join("data/pld/pl00.pld"))]);
    assert!(
        info.contains("geometry: 1 objects, 1 meshes, 4 vertices"),
        "{info}"
    );

    let export = dmc(&["export", s(&extracted), s(&root.join("export"))]);
    assert!(
        export.contains("2 converted (0 flagged for review), 0 failed"),
        "{export}"
    );
    // An independent glTF implementation must accept what we wrote.
    let (doc, buffers, images) =
        gltf::import(root.join("export/data/emd/em00.glb")).expect("valid glTF");
    assert_eq!(doc.meshes().count(), 1);
    assert_eq!(images.len(), 1);
    let prim = doc.meshes().next().unwrap().primitives().next().unwrap();
    let reader = prim.reader(|b| Some(&buffers[b.index()]));
    assert_eq!(reader.read_indices().unwrap().into_u32().count(), 6);

    let tex = dmc(&[
        "tex",
        s(&install.join("data/dmc1/loose.pws")),
        s(&root.join("tex")),
    ]);
    assert!(tex.starts_with("1 images"));

    let _ = fs::remove_dir_all(&root);
}

/// A synthetic room: two placed objects, a quad strip each, and a PC texture
/// container in section 34.
fn pc_room() -> Vec<u8> {
    use dmc_formats::dds;
    use dmc_formats::room::{self, NewRoomMesh, NewRoomObject, flag};
    let f = flag::ALWAYS;
    let object = |x: f32| {
        let mut transform = [[0.0; 4]; 4];
        for (i, row) in transform.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        transform[0][3] = x;
        NewRoomObject {
            transform,
            bounds: [x as i16 + 1, x as i16, 1, 0, 0, 0],
            meshes: vec![NewRoomMesh {
                tex_index: 0,
                positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]],
                normals: vec![[0., 0., 1.]; 4],
                uvs: vec![[0., 0.]; 4],
                flags: vec![f | flag::RESTART, f | flag::RESTART, f | flag::FRONT, f],
                colours: vec![[128, 128, 128]; 4],
            }],
        }
    };
    let geometry = room::build_geometry(1, &[object(0.0), object(10.0)]);
    let red = [0x00, 0xF8, 0, 0, 0, 0, 0, 0];
    let textures = texture::build_pc(
        texture::Kind::T32,
        &[dds::build(4, 4, dds::Format::Dxt1, &[&red])],
    );
    room::build(&geometry, Some(&textures))
}

#[test]
fn pc_room_pipeline() {
    let root = std::env::temp_dir().join(format!("opendmc-smoke-room-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let install = root.join("install");
    fs::create_dir_all(&install).unwrap();
    let nbz = install.join("dmc1-0.nbz");
    write_nbz(
        &nbz,
        &[("Fsd/r002.fsd", pc_room()), ("Pld/pl00.pld", pc_model())],
    );

    let reports = root.join("reports");
    dmc(&["inventory", s(&install), "--out", s(&reports)]);
    let md = fs::read_to_string(reports.join("inventory.md")).unwrap();
    assert!(
        md.contains("Files parsed as rooms: **1** (8 vertices)"),
        "{md}"
    );

    let spec = format!("{}::Fsd/r002.fsd", nbz.display());
    let info = dmc(&["info", &spec]);
    assert!(
        info.contains("Room { objects: 2, meshes: 2, vertices: 8, textures: 1 }"),
        "{info}"
    );

    let one = root.join("one.glb");
    let out = dmc(&["room", &spec, s(&one)]);
    assert!(
        out.contains("objects=  2") && out.contains("bounds=2/2"),
        "{out}"
    );

    let all = dmc(&["room", s(&nbz), s(&root.join("rooms"))]);
    assert!(
        all.contains("1 rooms converted (0 flagged for review), 0 failed"),
        "{all}"
    );
    let (doc, _, images) = gltf::import(root.join("rooms/Fsd/r002.glb")).expect("valid glTF");
    assert_eq!(doc.nodes().count(), 2);
    assert_eq!(images.len(), 1);
    let (t, _, _) = doc.nodes().nth(1).unwrap().transform().decomposed();
    assert_eq!(t, [10.0, 0.0, 0.0]);
    let _ = fs::remove_dir_all(&root);
}
