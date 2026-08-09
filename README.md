# norm_note

A Markdown vault that is actually yours — with a database engine that is
actually fast, and mobile that is actually as good as desktop.

No LLM. No telemetry. No server. No subscription.

> **Status: pre-alpha.** Syncing works end to end and is covered by a large
> test suite, including a simulation of five years of use by five people with
> different habits across nine devices. The cryptography has not been
> independently audited and the on-disk format will change before 1.0. Keep a
> backup of anything you point it at.

---

## What v0.1 is

A sync daemon and CLI — no GUI — that keeps a plain Markdown vault in sync
through **a folder you already own**: Dropbox, iCloud Drive, a NAS mount, an
external disk. It works on an existing Obsidian vault without modifying it.

The single sentence it has to earn:

> *Put your vault in Dropbox and never get a `(conflicted copy)` again.*

```bash
normd init ~/notes --target ~/Dropbox/norm     # prints your recovery phrase
normd watch ~/notes                            # the daemon
normd status ~/notes                           # what is and is not synced
```

On a second machine:

```bash
normd join ~/notes --target ~/Dropbox/norm --recovery 1450P-30D1R-...
normd sync ~/notes
```

## The three guarantees

**G1 — Zero Egress.** This software never connects to any host we control. No
telemetry, no crash reporting, no analytics, no update check, no online licence
validation, no fonts from a CDN, no model API. v0.1 contains no networking code
at all — the only transport is the local filesystem. This is enforced by the
dependency graph in [`deny.toml`](deny.toml) and checked in CI, so an accidental
`cargo add reqwest` fails the build rather than getting caught in review.

**G2 — Outlive the vendor.** The on-disk format is Markdown and YAML. The sync
protocol is specified in the open: [`docs/FORMAT.md`](docs/FORMAT.md). The core
is open source. If this project stops shipping, your vault keeps working.

**G3 — It's just files.** Delete the app and every note still opens in Notepad.
Edit a note in vim while the daemon is stopped and it merges correctly. Delete
`.norm/` entirely and you lose history, never a note.

## How it stays safe in a syncing folder

Cloud folders resolve concurrent writes to the same path by last-write-wins, or
by dropping a `(conflicted copy)` beside it. That is exactly how Obsidian
vaults get mangled.

norm_note never gives the folder a conflict to resolve. Every chunk of the
operation log is written to a path containing the id of the device that wrote
it, so no two devices ever write the same path. Chunks are immutable. The only
thing the folder has to do correctly is notice a new file — the one operation
every one of these products already gets right. Merging happens locally, in
[Automerge](https://automerge.org/), where it cannot lose text.

Delay is therefore harmless. If your laptop is closed for eight hours, the sync
is *late*, never lossy. That is what lets this work within iOS's background
execution limits instead of fighting them.

## Cryptography

⚠️ **Not independently audited.** Chunks are encrypted with XChaCha20-Poly1305
using implementations from the RustCrypto project; we do not implement any
primitive ourselves. The way those pieces are *assembled* has not been reviewed
by an external expert. An audit is planned and funded work. Until then, this
notice stays here.

The vault key lives outside the vault, in your user configuration directory —
because a key stored in `.norm/` would be uploaded to the same cloud folder as
the ciphertext it protects. It is not itself encrypted at rest; see
[`docs/FORMAT.md`](docs/FORMAT.md) §5.

## What v0.1 does not do

Stated plainly rather than discovered later:

- **No deletion.** A file you remove is written back. Guessing that a missing
  file means "delete everywhere" is unrecoverable when the guess is wrong.
- **No LAN peer-to-peer sync yet** — the only transport is a shared folder.
- **No GUI, no mobile app, no search, no queries.**
- **`watch` polls** every few seconds rather than watching the filesystem.
- **The recovery phrase is not BIP39.** It is checksummed and typo-resistant,
  but it is not twenty-four words.

## Layout

```
crates/norm-core   vault, oplog, CRDT, reconciliation — no networking
crates/normd       daemon and CLI
crates/norm-net    does not exist yet; when it does, the only crate
                   permitted to open a socket
docs/FORMAT.md     the on-disk and sync specification
```

## Building

```bash
cargo test --workspace
cargo deny check bans licenses advisories
```

The slower measurements and the five-year simulation are excluded from a normal
run:

```bash
cargo test --release --test scale    -- --ignored --nocapture
cargo test --release --test personas -- --ignored --nocapture
```

## Licence

Core: MPL-2.0.
