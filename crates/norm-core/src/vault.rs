//! Reading and writing the user's Markdown files.
//!
//! # Round-trip fidelity
//!
//! Guarantee G3 says the files are the user's, not ours. That makes
//! *not mangling them* a correctness requirement, not a nicety.
//!
//! Parsing YAML and re-serialising it silently reorders keys, drops comments,
//! rewrites quoting and normalises dates. A user who opens a note in norm_note
//! and never edits it must get a byte-identical file back. So [`Frontmatter`]
//! keeps the original text alongside the parsed view and only regenerates the
//! text when a field was actually changed.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::doc::DocId;
use crate::{Error, Result, NORM_DIR};

/// The user's folder of Markdown files.
///
/// Everything here treats the files as the authority. Nothing is renamed,
/// reformatted or reorganised, and nothing outside `.norm/` is written unless
/// the content actually changed.
pub struct Vault {
    root: PathBuf,
}

impl Vault {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path_of(&self, id: &DocId) -> PathBuf {
        let mut p = self.root.clone();
        for part in id.as_str().split('/') {
            p.push(part);
        }
        p
    }

    /// Every Markdown file in the vault.
    ///
    /// `.norm/` is skipped, as is anything starting with a dot: `.git`,
    /// `.obsidian` and friends are not the user's notes and must not be
    /// rewritten by us.
    pub fn scan(&self) -> Result<Vec<DocId>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }

        let mut ids = Vec::new();
        for entry in walkdir::WalkDir::new(&self.root)
            .into_iter()
            // Depth 0 is the vault root itself, and the filter must not apply
            // to it. A vault at `~/.notes` is a dot-directory by its own name;
            // pruning it would make the whole vault invisible rather than
            // skipping something inside it.
            .filter_entry(|e| e.depth() == 0 || !is_hidden(e.file_name()))
        {
            let entry =
                entry.map_err(|e| {
                    Error::Io(e.into_io_error().unwrap_or_else(|| {
                        std::io::Error::other("could not walk the vault directory")
                    }))
                })?;

            if !entry.file_type().is_file() {
                continue;
            }
            if entry.path().extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&self.root) else {
                continue;
            };
            ids.push(DocId::from_relative_path(rel));
        }

        ids.sort();
        Ok(ids)
    }

    pub fn read(&self, id: &DocId) -> Result<Option<String>> {
        match fs::read_to_string(self.path_of(id)) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes a note, replacing it atomically.
    ///
    /// A half-written note is worse than a stale one: an editor or a backup
    /// tool could pick up the truncated version. Writing to a temporary file
    /// and renaming means readers only ever see a complete file.
    ///
    /// # No fsync, on purpose
    ///
    /// Flushing each note cost about 4 ms, which made writing out a 5,000-note
    /// vault take 21 seconds — and note files are derived data. The oplog is
    /// the durable record; a file lost to a power cut can simply be written
    /// again from the CRDT.
    ///
    /// That is only safe because the caller says what it is about to write
    /// before writing it. `Workspace` journals the batch, so a crash leaves a
    /// record that these files may be torn, and recovery rewrites them from the
    /// CRDT instead of mistaking the damage for something the user typed.
    /// Without that journal, dropping the fsync here would be a data-loss bug.
    pub fn write(&self, id: &DocId, text: &str) -> Result<()> {
        let path = self.path_of(id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let tmp = {
            let mut s = path.clone().into_os_string();
            s.push(".norm-tmp");
            PathBuf::from(s)
        };

        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        drop(f);

        // Windows `rename` fails when the destination exists; `fs::rename` maps
        // to MoveFileEx with replace semantics, so this works on both.
        fs::rename(&tmp, &path)?;
        Ok(())
    }
}

fn is_hidden(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|n| n.starts_with('.') && n != "." && n != "..")
        || name == NORM_DIR
}

/// YAML frontmatter, retaining the exact source text until it is modified.
#[derive(Debug, Clone, PartialEq)]
pub struct Frontmatter {
    /// Original text between the `---` fences, without the fences themselves.
    /// `None` when the file had no frontmatter block at all.
    raw: Option<String>,
    parsed: serde_yaml_ng::Mapping,
    dirty: bool,
}

impl Frontmatter {
    pub fn empty() -> Self {
        Self {
            raw: None,
            parsed: serde_yaml_ng::Mapping::new(),
            dirty: false,
        }
    }

    pub fn get(&self, key: &str) -> Option<&serde_yaml_ng::Value> {
        self.parsed
            .get(serde_yaml_ng::Value::String(key.to_owned()))
    }

    pub fn set(&mut self, key: &str, value: serde_yaml_ng::Value) {
        self.parsed
            .insert(serde_yaml_ng::Value::String(key.to_owned()), value);
        self.dirty = true;
    }

    pub fn is_empty(&self) -> bool {
        self.parsed.is_empty() && self.raw.is_none()
    }

    /// Renders the block back to text, preserving the original byte-for-byte
    /// when nothing was modified.
    ///
    /// Fallible on purpose. Swallowing a serialisation failure here would drop
    /// the user's entire frontmatter block with no error and no trace — the
    /// exact class of silent data loss this project exists to avoid. Rare is
    /// not the same as acceptable.
    fn render(&self) -> Result<Option<String>> {
        match (&self.raw, self.dirty) {
            (Some(raw), false) => Ok(Some(raw.clone())),
            (_, _) if self.parsed.is_empty() => Ok(None),
            _ => serde_yaml_ng::to_string(&self.parsed)
                .map(Some)
                .map_err(|e| Error::Frontmatter {
                    path: "<in memory>".to_owned(),
                    reason: format!("could not serialise frontmatter: {e}"),
                }),
        }
    }
}

/// A parsed note. `body` is the Markdown after the frontmatter block, verbatim.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub frontmatter: Frontmatter,
    pub body: String,
    /// Line ending observed in the source, reused on write so we do not flip a
    /// user's whole file between CRLF and LF and produce a spurious diff.
    pub newline: Newline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Newline {
    Lf,
    Crlf,
}

impl Newline {
    pub fn as_str(self) -> &'static str {
        match self {
            Newline::Lf => "\n",
            Newline::Crlf => "\r\n",
        }
    }

    fn detect(s: &str) -> Self {
        match s.find('\n') {
            Some(i) if i > 0 && s.as_bytes()[i - 1] == b'\r' => Newline::Crlf,
            _ => Newline::Lf,
        }
    }
}

impl Note {
    /// Parses note text. Never fails on missing frontmatter — a plain Markdown
    /// file is a valid note with an empty frontmatter block.
    pub fn parse(path: &Path, text: &str) -> Result<Self> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let newline = Newline::detect(text);

        let Some((raw, body)) = split_frontmatter(text) else {
            return Ok(Note {
                frontmatter: Frontmatter::empty(),
                body: text.to_owned(),
                newline,
            });
        };

        let parsed: serde_yaml_ng::Mapping = if raw.trim().is_empty() {
            serde_yaml_ng::Mapping::new()
        } else {
            serde_yaml_ng::from_str(raw).map_err(|e| Error::Frontmatter {
                path: path.display().to_string(),
                reason: e.to_string(),
            })?
        };

        Ok(Note {
            frontmatter: Frontmatter {
                raw: Some(raw.to_owned()),
                parsed,
                dirty: false,
            },
            body: body.to_owned(),
            newline,
        })
    }

    pub fn to_text(&self) -> Result<String> {
        let nl = self.newline.as_str();
        Ok(match self.frontmatter.render()? {
            Some(fm) => {
                let fm = fm.trim_end_matches(['\n', '\r']);
                format!("---{nl}{fm}{nl}---{nl}{}", self.body)
            }
            None => self.body.clone(),
        })
    }

    /// Stable content address, used for history snapshots and change detection.
    pub fn content_hash(&self) -> Result<blake3::Hash> {
        Ok(blake3::hash(self.to_text()?.as_bytes()))
    }
}

/// Splits `---\n...\n---\n` off the front. Returns `(yaml, body)`.
///
/// Returns `None` unless the very first line is a `---` fence *and* a closing
/// fence exists. An unterminated fence is treated as body text, matching what
/// every other Markdown tool does — silently swallowing the rest of the file
/// would be a data-loss bug.
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let after_open = strip_fence_line(text)?;

    let mut offset = 0usize;
    for line in after_open.split_inclusive('\n') {
        if is_fence(line) {
            let yaml = &after_open[..offset];
            let body = &after_open[offset + line.len()..];
            return Some((yaml, body));
        }
        offset += line.len();
    }

    None
}

fn strip_fence_line(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("---")?;
    if let Some(r) = rest.strip_prefix("\r\n") {
        Some(r)
    } else {
        rest.strip_prefix('\n')
    }
}

fn is_fence(line: &str) -> bool {
    let t = line.trim_end_matches(['\n', '\r']);
    t == "---" || t == "..."
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p() -> PathBuf {
        PathBuf::from("note.md")
    }

    #[test]
    fn plain_markdown_has_no_frontmatter() {
        let n = Note::parse(&p(), "# hello\n\nbody\n").unwrap();
        assert!(n.frontmatter.is_empty());
        assert_eq!(n.body, "# hello\n\nbody\n");
    }

    #[test]
    fn parses_frontmatter_and_body() {
        let n = Note::parse(&p(), "---\ntitle: hi\ntags: [a, b]\n---\nbody\n").unwrap();
        assert_eq!(
            n.frontmatter.get("title"),
            Some(&serde_yaml_ng::Value::String("hi".into()))
        );
        assert_eq!(n.body, "body\n");
    }

    #[test]
    fn unterminated_fence_is_body_not_frontmatter() {
        // Swallowing this as frontmatter would lose the whole file.
        let src = "---\ntitle: hi\nbody with no closing fence\n";
        let n = Note::parse(&p(), src).unwrap();
        assert!(n.frontmatter.is_empty());
        assert_eq!(n.body, src);
    }

    #[test]
    fn round_trips_byte_identically_when_untouched() {
        // The G3 test: open a note, change nothing, get the same bytes back.
        for src in [
            "---\nb: 2\na: 1\n---\nbody\n",        // key order preserved
            "---\ntitle: \"quoted\"\n---\nbody\n", // quoting preserved
            "---\n# a comment\nk: v\n---\nbody\n", // comments preserved
            "---\r\ntitle: hi\r\n---\r\nbody\r\n", // CRLF preserved
            "# no frontmatter\n",
            "",
        ] {
            let n = Note::parse(&p(), src).unwrap();
            assert_eq!(n.to_text().unwrap(), src, "round-trip changed: {src:?}");
        }
    }

    #[test]
    fn bom_is_stripped() {
        let n = Note::parse(&p(), "\u{feff}---\nk: v\n---\nbody\n").unwrap();
        assert_eq!(n.body, "body\n");
    }

    #[test]
    fn empty_frontmatter_block_is_valid() {
        let n = Note::parse(&p(), "---\n---\nbody\n").unwrap();
        assert!(n.frontmatter.parsed.is_empty());
        assert_eq!(n.body, "body\n");
    }

    #[test]
    fn editing_a_field_regenerates_the_block() {
        let mut n = Note::parse(&p(), "---\na: 1\n---\nbody\n").unwrap();
        n.frontmatter
            .set("a", serde_yaml_ng::Value::Number(2.into()));
        assert!(n.to_text().unwrap().contains("a: 2"));
    }
}
