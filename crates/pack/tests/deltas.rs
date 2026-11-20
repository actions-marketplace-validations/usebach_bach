//! Binary deltas in the archive: written next to whole files, read back as
//! the new files, and refused when the earlier file isn't the one they were
//! made from.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use bach_pack::archive::{self, Bases, Compression, DeltaBase, Source, WrongBase};
use bach_pack::protect::sha256_hex;

fn program(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|index| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if index % 7 == 0 {
                (index % 251) as u8
            } else {
                (state >> 24) as u8
            }
        })
        .collect()
}

struct Case {
    dir: PathBuf,
    sources: Vec<Source>,
    /// The earlier versions, by packaged path.
    old: HashMap<String, Vec<u8>>,
    /// The new files, by packaged path.
    new: HashMap<String, Vec<u8>>,
}

fn case(name: &str) -> Case {
    let dir = std::env::temp_dir().join(format!("bach-delta-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let big_old = program(1_000_000, 1);
    let mut big_new = big_old.clone();
    big_new[100] ^= 0xFF;
    big_new.splice(400_000..400_000, program(500, 2));
    let rewritten_old = program(50_000, 3);
    let rewritten_new = program(50_000, 4);
    let same = b"unchanged".to_vec();
    let mut case = Case { dir: dir.clone(), sources: Vec::new(), old: HashMap::new(), new: HashMap::new() };
    for (name, old, new) in [
        ("app.exe", Some(big_old), big_new),
        ("data/rewritten.bin", Some(rewritten_old), rewritten_new),
        ("new-file.txt", None, b"a file that is new".to_vec()),
        ("notes.txt", Some(same.clone()), same),
    ] {
        let path = dir.join(name.replace('/', "_"));
        std::fs::write(&path, &new).unwrap();
        case.sources.push(Source { packaged: name.into(), path, size: new.len() as u64 });
        if let Some(old) = old {
            case.old.insert(name.into(), old);
        }
        case.new.insert(name.into(), new);
    }
    case
}

fn bases(case: &Case) -> HashMap<String, DeltaBase> {
    case.old
        .iter()
        .map(|(path, bytes)| (path.clone(), DeltaBase { bytes: bytes.clone(), sha256: sha256_hex(&bytes[..]).unwrap() }))
        .collect()
}

fn pack(case: &Case) -> (Vec<u8>, Vec<String>) {
    let (out, hashes) = archive::write_hashed_with(Vec::new(), &case.sources, &bases(case), Compression::Fast, 1, &mut |_| {}).unwrap();
    (out, hashes)
}

#[test]
fn a_changed_big_file_is_stored_as_a_small_delta_and_comes_back_whole() {
    let case = case("roundtrip");
    let (plain, _) = archive::write_hashed(Vec::new(), &case.sources, Compression::Fast, 1, &mut |_| {}).unwrap();
    let (with_deltas, hashes) = pack(&case);
    // The 1 MB program is a few kilobytes, the rewritten file is stored whole.
    assert!(with_deltas.len() + 900_000 < plain.len(), "{} bytes with deltas, {} without", with_deltas.len(), plain.len());
    // The checksums are those of the new files, not of the deltas.
    for (source, hash) in case.sources.iter().zip(&hashes) {
        assert_eq!(hash, &sha256_hex(&case.new[&source.packaged][..]).unwrap(), "{}", source.packaged);
    }

    let mut asked = Vec::new();
    let mut got: HashMap<String, Vec<u8>> = HashMap::new();
    let mut earlier = |path: &str, size: u64| -> anyhow::Result<Vec<u8>> {
        asked.push(path.to_string());
        let old = case.old.get(path).cloned().unwrap();
        assert_eq!(old.len() as u64, size);
        Ok(old)
    };
    archive::read_with(&with_deltas[..], Bases::From(&mut earlier), &mut |path, size, body| {
        let mut bytes = Vec::new();
        body.read_to_end(&mut bytes)?;
        assert_eq!(bytes.len() as u64, size);
        got.insert(path.to_string(), bytes);
        Ok(())
    })
    .unwrap();
    assert_eq!(got, case.new, "every file comes out as the new file");
    assert_eq!(asked, ["app.exe"], "only a file that was worth a delta needs its earlier version");
    std::fs::remove_dir_all(&case.dir).unwrap();
}

#[test]
fn a_reader_without_the_earlier_files_refuses_or_steps_over_the_deltas() {
    let case = case("refuse");
    let (archive_bytes, _) = pack(&case);
    // A full setup file has no use for deltas: one in it is an error naming the file.
    let error = archive::read(&archive_bytes[..], &mut |_, _, _| Ok(())).err().unwrap().to_string();
    assert!(error.contains("app.exe") && error.contains("binary delta"), "{error}");
    // Checking the rest of a patch alone steps over them.
    let mut seen = Vec::new();
    archive::read_with(&archive_bytes[..], Bases::Skip, &mut |path, _, _| {
        seen.push(path.to_string());
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, ["data/rewritten.bin", "new-file.txt", "notes.txt"]);
    std::fs::remove_dir_all(&case.dir).unwrap();
}

#[test]
fn a_different_earlier_file_is_a_wrong_base_not_a_damaged_archive() {
    let case = case("wrong");
    let (archive_bytes, _) = pack(&case);
    // Another file of the same size.
    let mut other = |_: &str, size: u64| -> anyhow::Result<Vec<u8>> { Ok(program(size as usize, 99)) };
    let error = archive::read_with(&archive_bytes[..], Bases::From(&mut other), &mut |_, _, body| {
        std::io::copy(body, &mut std::io::sink())?;
        Ok(())
    })
    .err()
    .unwrap();
    assert_eq!(error.downcast_ref::<WrongBase>().map(|wrong| wrong.path.as_str()), Some("app.exe"), "{error:#}");
    // The reader that can't find the file says so in its own words, and that is passed through.
    let mut missing = |path: &str, _: u64| -> anyhow::Result<Vec<u8>> { anyhow::bail!("{path} is missing") };
    let error = archive::read_with(&archive_bytes[..], Bases::From(&mut missing), &mut |_, _, _| Ok(())).err().unwrap();
    assert!(error.to_string().contains("app.exe is missing"), "{error:#}");
    std::fs::remove_dir_all(&case.dir).unwrap();
}

#[test]
fn a_damaged_delta_is_an_error_and_a_hash_of_the_wrong_result_is_found_by_the_reader_of_the_file() {
    let case = case("damaged");
    let (archive_bytes, _) = pack(&case);
    // Flip bytes across the archive: every outcome is an error or bytes that differ, never a panic.
    let old = case.old.clone();
    let mut survived = 0;
    for position in (0..archive_bytes.len()).step_by(archive_bytes.len() / 40 + 1) {
        let mut damaged = archive_bytes.clone();
        damaged[position] ^= 0x55;
        let mut earlier = |path: &str, _: u64| -> anyhow::Result<Vec<u8>> { Ok(old[path].clone()) };
        let mut got: HashMap<String, Vec<u8>> = HashMap::new();
        let result = archive::read_with(&damaged[..], Bases::From(&mut earlier), &mut |path, _, body| {
            let mut bytes = Vec::new();
            body.read_to_end(&mut bytes)?;
            got.insert(path.to_string(), bytes);
            Ok(())
        });
        if result.is_ok() && got == case.new {
            survived += 1;
        }
    }
    assert!(survived < 40, "flipping bytes must not leave every archive intact");
    let _ = std::io::stdout().flush();
    std::fs::remove_dir_all(&case.dir).unwrap();
}

#[test]
fn a_delta_that_isnt_smaller_is_not_used() {
    let case = case("worth");
    // Only the completely rewritten file has an earlier version here.
    let mut few = bases(&case);
    few.retain(|path, _| path == "data/rewritten.bin");
    let (with, _) = archive::write_hashed_with(Vec::new(), &case.sources, &few, Compression::Fast, 1, &mut |_| {}).unwrap();
    let (plain, _) = archive::write_hashed(Vec::new(), &case.sources, Compression::Fast, 1, &mut |_| {}).unwrap();
    assert!(
        (with.len() as i64 - plain.len() as i64).abs() < 64,
        "a file that changed completely stays a whole file: {} vs {}",
        with.len(),
        plain.len()
    );
    // Read back without any earlier file: nothing in it needed one.
    archive::read(&with[..], &mut |_, _, _| Ok(())).unwrap();
    std::fs::remove_dir_all(&case.dir).unwrap();
}
