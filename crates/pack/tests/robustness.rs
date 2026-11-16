//! Untrusted input must never panic or hang: a setup file's container, its
//! archive and manifest, an update feed and a config are all read from places
//! other people control. Each test takes valid input, damages it thousands of
//! ways with a fixed seed, and requires that reading it ends in `Ok` or `Err`.
//!
//! This is a deterministic stand-in for a fuzzer: it runs in every `cargo test`
//! and a failure names the seed and the iteration that reproduces it. The
//! `fuzz` folder holds cargo-fuzz targets for long runs.

use std::fs::File;
use std::io::{Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use bach_pack::archive::{self, Compression, Source};
use bach_pack::{config, container, feedsig};

/// A small deterministic random source (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, limit: usize) -> usize {
        if limit == 0 {
            0
        } else {
            (self.next() % limit as u64) as usize
        }
    }
}

/// Damages `input` in one of several ways, favouring its start and its end,
/// where headers and trailers live.
fn mutate(input: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut bytes = input.to_vec();
    let place = |rng: &mut Rng, len: usize| -> usize {
        match rng.below(4) {
            0 => rng.below(len.min(700)),
            1 | 2 => len.saturating_sub(1 + rng.below(len.min(300))),
            _ => rng.below(len),
        }
    };
    for _ in 0..1 + rng.below(6) {
        if bytes.is_empty() {
            bytes.push(rng.next() as u8);
            continue;
        }
        let at = place(rng, bytes.len());
        match rng.below(8) {
            0 => bytes[at] ^= 1 << rng.below(8),
            1 => bytes[at] = rng.next() as u8,
            2 => bytes.truncate(at),
            3 => bytes.extend((0..rng.below(64)).map(|_| rng.next() as u8)),
            4 => {
                let end = (at + rng.below(32)).min(bytes.len());
                bytes[at..end].fill(0);
            }
            5 => {
                let end = (at + 1 + rng.below(8)).min(bytes.len());
                bytes[at..end].fill(0xFF);
            }
            6 => {
                let end = (at + rng.below(16)).min(bytes.len());
                bytes.drain(at..end);
            }
            _ => {
                let piece: Vec<u8> = bytes[at..(at + rng.below(24)).min(bytes.len())].to_vec();
                bytes.splice(at..at, piece);
            }
        }
    }
    bytes
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bach-robust-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `body` and fails the test with the case that broke it if it panics.
fn must_not_panic(what: &str, seed: u64, iteration: usize, body: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(body)).is_err() {
        panic!("{what} panicked on damaged input (seed {seed}, iteration {iteration})");
    }
}

/// How many damaged inputs each test tries: 300 by default, so `cargo test`
/// stays quick; `BACH_FUZZ_ITERATIONS=20000` makes a long run.
fn iterations() -> usize {
    std::env::var("BACH_FUZZ_ITERATIONS").ok().and_then(|value| value.parse().ok()).unwrap_or(300)
}

/// A real archive of a few files.
fn archive_bytes(dir: &Path) -> Vec<u8> {
    let mut sources = Vec::new();
    for (name, content) in
        [("a.txt", b"hello".to_vec()), ("sub/b.bin", vec![7u8; 5000]), ("c.dll", (0..=255u8).cycle().take(3000).collect())]
    {
        let path = dir.join(name.replace('/', "_"));
        std::fs::write(&path, &content).unwrap();
        sources.push(Source { packaged: name.into(), path, size: content.len() as u64 });
    }
    archive::write(Vec::new(), &sources, Compression::Fast, &mut |_| {}).unwrap()
}

#[test]
fn a_damaged_archive_is_an_error_not_a_panic() {
    let dir = temp("archive");
    let good = archive_bytes(&dir);
    let mut seen = 0;
    archive::read(&good[..], &mut |_, _, body| {
        seen += std::io::copy(body, &mut std::io::sink())?;
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, 5 + 5000 + 3000);

    let seed = 0xA5C1_17E5;
    let mut rng = Rng(seed);
    for iteration in 0..iterations() {
        let damaged = mutate(&good, &mut rng);
        must_not_panic("the archive reader", seed, iteration, || {
            let _ = archive::read(&damaged[..], &mut |_name, _size, body| {
                // Read at most a little: a lie about the size must not make this read forever.
                let mut sink = Vec::new();
                body.take(1 << 20).read_to_end(&mut sink)?;
                Ok(())
            });
        });
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A config project, and a setup-like file built from it.
fn project(dir: &Path) -> (PathBuf, config::Project) {
    std::fs::create_dir_all(dir.join("build")).unwrap();
    std::fs::write(dir.join("build/hello.exe"), b"MZ").unwrap();
    std::fs::write(dir.join("build/data.txt"), b"data").unwrap();
    let toml = "[app]\nname = \"Hello\"\nversion = \"1.0.0\"\npublisher = \"Example\"\nexecutable = \"hello.exe\"\n\n[files]\nsource = \"build\"\n\n[updates]\nfeed = \"https://example.com/latest.json\"\n";
    let path = dir.join("bach.toml");
    std::fs::write(&path, toml).unwrap();
    let project = config::load(&path).unwrap();
    (path, project)
}

#[test]
fn a_damaged_setup_file_is_an_error_not_a_panic() {
    let dir = temp("container");
    let (_, project) = project(&dir);
    // The head of a real executable stands in for the runtime, so the PE
    // headers the container reads are real.
    let exe = std::fs::read(std::env::current_exe().unwrap()).unwrap();
    let runtime = &exe[..4096.min(exe.len())];
    let good = dir.join("good.exe");
    {
        let mut file = File::create(&good).unwrap();
        file.write_all(runtime).unwrap();
        let archive = archive_bytes(&dir);
        file.write_all(&archive).unwrap();
        drop(file);
        let mut file = std::fs::OpenOptions::new().read(true).write(true).open(&good).unwrap();
        container::finish(&mut file, runtime.len() as u64, b"<svg/>", &project.manifest).unwrap();
    }
    let original = std::fs::read(&good).unwrap();
    {
        let mut file = File::open(&good).unwrap();
        let layout = container::locate(&mut file).unwrap().expect("the undamaged file has a payload");
        assert_eq!(container::read_manifest(&mut file, &layout).unwrap().app.name, "Hello");
    }

    let seed = 0xC0DE_0001;
    let mut rng = Rng(seed);
    let target = dir.join("damaged.exe");
    for iteration in 0..iterations() {
        std::fs::write(&target, mutate(&original, &mut rng)).unwrap();
        must_not_panic("the container reader", seed, iteration, || {
            let Ok(mut file) = File::open(&target) else { return };
            let Ok(Some(layout)) = container::locate(&mut file) else { return };
            let _ = container::read_manifest(&mut file, &layout);
            let _ = container::read_logo(&mut file, &layout);
            let _ = container::payload_end(&mut file);
        });
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_damaged_manifest_is_refused_or_valid_never_a_panic() {
    let dir = temp("manifest");
    let (_, project) = project(&dir);
    let json = serde_json::to_vec(&project.manifest).unwrap();
    let seed = 0x3A71_F3E5;
    let mut rng = Rng(seed);
    for iteration in 0..iterations() * 2 {
        let damaged = mutate(&json, &mut rng);
        must_not_panic("the manifest check", seed, iteration, || {
            if let Ok(manifest) = serde_json::from_slice::<bach_pack::manifest::Manifest>(&damaged) {
                let _ = manifest.validate();
                // What a window or an uninstaller derives from it must hold up too.
                let _ = manifest.as_installed(&bach_pack::manifest::Choices::defaults(&manifest));
            }
        });
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_damaged_feed_is_refused_never_a_panic() {
    let (private, public) = feedsig::generate().unwrap();
    let mut entry = bach_pack::manifest::FeedEntry {
        version: "2.0.0".into(),
        url: "https://example.com/a.exe".into(),
        sha256: "ab".repeat(32),
        notes: Some("notes".into()),
        size: Some(9),
        rollout: Some(10),
        delta: None,
        signature: None,
    };
    entry.signature = Some(feedsig::sign(&private, "Hello", &entry).unwrap());
    let json = serde_json::to_vec(&entry).unwrap();
    let seed = 0xFEED_0001;
    let mut rng = Rng(seed);
    for iteration in 0..iterations() * 2 {
        let damaged = mutate(&json, &mut rng);
        must_not_panic("the feed reader", seed, iteration, || {
            if let Ok(entry) = serde_json::from_slice::<bach_pack::manifest::FeedEntry>(&damaged) {
                let _ = feedsig::verify(&public, "Hello", &entry);
                let _ = bach_pack::version::compare(&entry.version, "1.0.0");
            }
        });
    }
    // The undamaged entry still verifies, so the loop above tested something.
    assert!(feedsig::verify(&public, "Hello", &entry).is_ok());
}

#[test]
fn a_damaged_config_is_a_sentence_never_a_panic() {
    let dir = temp("config");
    let (path, _) = project(&dir);
    let toml = std::fs::read(&path).unwrap();
    let seed = 0x7041_C0F1;
    let mut rng = Rng(seed);
    for iteration in 0..iterations() {
        let damaged = String::from_utf8_lossy(&mutate(&toml, &mut rng)).into_owned();
        must_not_panic("the config reader", seed, iteration, || {
            if let Ok(project) = config::parse(&damaged, &dir) {
                let _ = project.manifest.validate();
            }
        });
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn damaged_paths_never_escape_or_panic() {
    let seed = 0x09A7_4511;
    let mut rng = Rng(seed);
    let base = b"bin/app.exe".to_vec();
    for iteration in 0..iterations() * 4 {
        let text = String::from_utf8_lossy(&mutate(&base, &mut rng)).into_owned();
        must_not_panic("the path rules", seed, iteration, || {
            if let Ok(normal) = bach_pack::paths::normalize(&text) {
                assert!(!normal.starts_with('/') && !normal.split('/').any(|part| part == ".." || part.is_empty()), "{normal:?}");
                let _ = bach_pack::paths::matches("**/*.exe", &normal);
            }
            let _ = bach_pack::paths::check_data_folder(&text);
            let _ = bach_pack::paths::is_valid_id(&text);
        });
    }
}
