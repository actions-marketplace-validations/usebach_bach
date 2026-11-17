//! The examples in the docs are real: every complete `bach.toml` in them has
//! to read without a mistake, and every link inside the docs has to land.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The Markdown files that make up the repository's docs.
fn pages() -> Vec<PathBuf> {
    let mut pages: Vec<PathBuf> = std::fs::read_dir(repo().join("docs"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    pages.push(repo().join("README.md"));
    pages.push(repo().join("CONTRIBUTING.md"));
    pages.sort();
    pages
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

/// Every ```toml block of a page, with the line it starts on.
fn toml_blocks(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((at, line)) = lines.next() {
        if line.trim_start().starts_with("```toml") {
            let mut block = String::new();
            for (_, inner) in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    break;
                }
                block.push_str(inner);
                block.push('\n');
            }
            found.push((at + 1, block));
        }
    }
    found
}

/// A block that is a whole config, not a piece of one.
fn is_whole_config(block: &str) -> bool {
    block.lines().any(|line| line.trim() == "[app]") && block.contains("executable") && block.contains("name")
}

#[test]
fn every_toml_example_in_the_docs_is_toml() {
    let mut count = 0;
    for page in pages() {
        for (line, block) in toml_blocks(&read(&page)) {
            count += 1;
            if let Err(err) = toml::from_str::<toml::Table>(&block) {
                panic!("{}:{line}: the example isn't TOML: {err}", page.display());
            }
        }
    }
    assert!(count > 20, "only {count} examples found: did the docs move?");
}

#[test]
fn every_whole_bach_toml_in_the_docs_reads() {
    let mut count = 0;
    for page in pages() {
        for (line, config) in toml_blocks(&read(&page)).into_iter().filter(|(_, block)| is_whole_config(block)) {
            count += 1;
            let dir = std::env::temp_dir().join(format!("bach-docs-{}-{count}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            // The example names an app to package: make its folder and program.
            let table: toml::Table = toml::from_str(&config).unwrap();
            let source = table.get("files").and_then(|files| files.get("source")).and_then(|v| v.as_str()).unwrap_or("app");
            let program = table["app"]["executable"].as_str().unwrap();
            std::fs::create_dir_all(dir.join(source)).unwrap();
            std::fs::write(dir.join(source).join(program), b"MZ").unwrap();
            // The files its components pick by pattern: one that fits each pattern.
            for component in table.get("components").and_then(|c| c.as_array()).into_iter().flatten() {
                for pattern in component.get("files").and_then(|f| f.as_array()).into_iter().flatten().filter_map(|p| p.as_str()) {
                    let concrete = pattern.replace("**/", "").replace("/**", "/file").replace('*', "x");
                    let path = dir.join(source).join(concrete.trim_end_matches('/'));
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    if !path.exists() {
                        std::fs::write(&path, b"x").unwrap();
                    }
                }
            }
            // And the other files it names, as the reader says they are missing.
            let mut result = bach_pack::config::parse(&config, &dir);
            for _ in 0..40 {
                let Err(err) = &result else { break };
                let message = format!("{err:#}");
                let Some(path) = missing(&message, &dir) else { break };
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                if message.contains("isn't a folder") {
                    std::fs::create_dir_all(&path).unwrap();
                } else {
                    std::fs::write(&path, b"x").unwrap();
                }
                result = bach_pack::config::parse(&config, &dir);
            }
            let _ = std::fs::remove_dir_all(&dir);
            if let Err(err) = result {
                panic!("{}:{line}: the example doesn't read: {err:#}", page.display());
            }
        }
    }
    assert!(count >= 3, "only {count} whole configs found: did the docs move?");
}

/// The file or folder an error says isn't there, when it's inside `dir`.
fn missing(message: &str, dir: &Path) -> Option<PathBuf> {
    let dir_text = dir.display().to_string();
    let start = message.find(&dir_text)?;
    let rest = &message[start..];
    let end = rest.find(" isn't a").or_else(|| rest.find(", but there's no such").map(|_| rest.len()))?;
    let path = PathBuf::from(&rest[..end]);
    path.starts_with(dir).then_some(path)
}

/// How GitHub names the anchor of a heading: lower case, spaces to `-`, and
/// only letters, digits, `-` and `_` kept.
fn slug(heading: &str) -> String {
    let heading = heading.trim().trim_start_matches('#').trim().replace('`', "");
    heading
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/// The anchors a page offers: one per heading (outside code blocks).
fn anchors(text: &str) -> Vec<String> {
    let mut in_code = false;
    let mut found: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
        } else if !in_code && line.starts_with('#') {
            let base = slug(line);
            let same = found
                .iter()
                .filter(|a| **a == base || a.starts_with(&format!("{base}-")) && a[base.len() + 1..].chars().all(|c| c.is_ascii_digit()))
                .count();
            found.push(if same == 0 { base } else { format!("{base}-{same}") });
        }
    }
    found
}

/// The `(target)` of every Markdown link in a page, outside code.
fn links(text: &str) -> Vec<(usize, String)> {
    let mut in_code = false;
    let mut found = Vec::new();
    for (at, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        let mut rest = line;
        while let Some(open) = rest.find("](") {
            rest = &rest[open + 2..];
            if let Some(close) = rest.find(')') {
                found.push((at + 1, rest[..close].trim().to_string()));
                rest = &rest[close..];
            }
        }
    }
    found
}

#[test]
fn every_link_inside_the_docs_lands() {
    let mut checked = 0;
    for page in pages() {
        let text = read(&page);
        for (line, target) in links(&text) {
            // Other sites aren't checked here: a test must not need a network.
            if target.contains("://") || target.starts_with("mailto:") || target.is_empty() {
                continue;
            }
            checked += 1;
            let (file, anchor) = match target.split_once('#') {
                Some((file, anchor)) => (file, Some(anchor)),
                None => (target.as_str(), None),
            };
            let destination = if file.is_empty() { page.clone() } else { page.parent().unwrap().join(file) };
            assert!(destination.exists(), "{}:{line}: the link ({target}) points at a file that isn't there", page.display());
            if let Some(anchor) = anchor {
                if destination.extension().is_some_and(|ext| ext == "md") {
                    let offered = anchors(&read(&destination));
                    assert!(
                        offered.iter().any(|a| a == anchor),
                        "{}:{line}: the link ({target}) points at a heading that isn't in {}",
                        page.display(),
                        destination.display()
                    );
                }
            }
        }
    }
    assert!(checked > 20, "only {checked} links found: did the docs move?");
}

#[test]
fn the_checks_themselves_see_a_mistake() {
    let page = "# Title\n\n## [access]\n\n## Same\n\n## Same\n\n```\n## not a heading\n```\n[a](#access) [b](#same-1) [c](#nope) [d](https://x.example/#y)\n";
    assert_eq!(anchors(page), ["title", "access", "same", "same-1"]);
    let targets: Vec<String> = links(page).into_iter().map(|(_, target)| target).collect();
    assert_eq!(targets, ["#access", "#same-1", "#nope", "https://x.example/#y"]);
    let offered = anchors(page);
    assert!(offered.contains(&"access".to_string()) && !offered.contains(&"nope".to_string()));
    assert!(toml::from_str::<toml::Table>("[app\nname = 1").is_err());
    assert!(!is_whole_config("[app]\ndescription = \"x\"\n"));
}

/// A number of languages written in the repository's texts, or in what
/// `bach init` writes, has to be the number Bach really has: it went stale
/// when five languages were added.
#[test]
fn the_number_of_languages_in_the_texts_is_the_real_one() {
    let real = bach_pack::i18n::LANGUAGES.len();
    let sources = [
        repo().join("README.md"),
        repo().join("AGENTS.md"),
        repo().join("skills/bach/SKILL.md"),
        repo().join("crates/cli/src/template.rs"),
    ];
    for path in sources {
        let text = read(&path);
        for (at, _) in text.match_indices(" language") {
            // "19 languages" and "all 19, following": the number just before the word, or before a comma.
            let before = &text[..at];
            let number: String = before.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect();
            if let Ok(number) = number.parse::<usize>() {
                assert_eq!(number, real, "{} says {number} languages, Bach has {real}", path.display());
            }
        }
        for (at, _) in text.match_indices("default: all ") {
            let number: String = text[at + "default: all ".len()..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(number) = number.parse::<usize>() {
                assert_eq!(number, real, "{} says all {number} languages, Bach has {real}", path.display());
            }
        }
    }
}
