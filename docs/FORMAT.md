# norm_note on-disk and sync format

**Version 1. Pre-alpha — this format will change before 1.0, and changes before
then may not be migrated.**

This document exists because of guarantee G2: *if this project stops shipping,
your vault keeps working.* A promise like that is worth nothing without a
specification someone else could implement from. This is that specification.

Everything here is derived from the code in `crates/norm-core`; where the two
disagree, the code is what runs and this document is the bug.

---

## 1. What is actually yours

```
vault/
├── **/*.md              your notes — nothing else is needed to read them
├── (any other files)    ignored, never touched
└── .norm/               derived state, safe to delete
```

**A vault is a folder of Markdown files.** `.norm/` holds indexes, sync state
and history. Deleting it loses the version history and forces a re-scan; it
loses no note.

Files whose name begins with `.` are skipped, at any depth — `.git`,
`.obsidian`, `.trash` and so on. The vault's own directory is exempt from that
rule, so a vault at `~/.notes` works.

Only `*.md` files are treated as notes.

---

## 2. Note identity

A note is identified by its path relative to the vault root, with:

- `/` as separator on every platform,
- **Unicode NFC** normalisation,
- case preserved and significant.

NFC matters: macOS stores filenames decomposed, Windows and Linux generally
composed. Without normalising, `café.md` written on one machine and synced to
another is two different notes, and the vault fills up with duplicates. Thai,
Vietnamese and Korean filenames hit the same problem.

Case is *not* folded. Windows and macOS are usually case-insensitive and Linux
is not; folding would merge two genuinely distinct files on Linux.

---

## 3. Document model

Each note is one [Automerge](https://automerge.org/) document containing a
single `Text` object at `ROOT["content"]` holding **the entire file**,
frontmatter fence included.

Storing frontmatter as a structured map would merge better key-by-key, and is
rejected: materialising a map back to YAML reorders keys, drops comments and
rewrites quoting, so syncing a note you never edited would rewrite your file.
Fidelity outranks merge elegance.

### 3.1 Genesis

Every replica of a note starts from a **byte-identical first change**:

- actor id: the 16 ASCII bytes `norm_note-gen-01`
- timestamp: `0`
- no message
- one operation: create a `Text` object at `ROOT["content"]`

This is not cosmetic. If each device created its content object locally, two
devices that independently create `meeting.md` would end up with two `Text`
objects competing for the same key; Automerge picks one, and everything written
into the loser stops being visible.

---

## 4. Chunks

The operation log is a set of immutable, individually encrypted files.

```
.norm/sync/ops/<device-hex>/<seq>.<ext>
```

| part | meaning |
|---|---|
| `device-hex` | 32 lowercase hex characters — the writing device's 16-byte id |
| `seq` | that device's sequence number, decimal, **zero-padded to 20 digits** |
| `ext` | `op` for a delta, `snap` for a snapshot |

Zero-padding to 20 digits (the width of `u64::MAX`) means a plain lexical
directory listing is already in sequence order, on every filesystem and object
store, without reading or parsing anything.

**No two devices ever write the same path.** This is the property the whole
design rests on: a folder-syncing product never has two writers for one path,
so it never has to pick a winner, and never produces a `(conflicted copy)`. The
only operation it must perform correctly is "a new file appeared".

Chunks are immutable. State moves forward by appending, never by rewriting.

### 4.1 Sequence numbers

A device's next sequence number is held in:

```
.norm/sync/state/<device-hex>.seq
```

It is incremented and flushed to disk **before** the chunk that uses it is
written. A crash therefore loses a sequence number rather than reissuing one.

**Gaps are legal.** A reader must treat the sequence as strictly increasing but
not contiguous, and must never wait for a missing number — doing so would stall
sync permanently after one badly-timed power cut.

This matters more than it looks: see §5.2.

### 4.2 Chunk file layout

```
+--------------------------------+
| header, 55 bytes, plaintext    |
+--------------------------------+
| ciphertext                     |
+--------------------------------+
```

Header, all integers little-endian:

| offset | size | field |
|---|---|---|
| 0 | 4 | magic, ASCII `NRM1` |
| 4 | 2 | format version, `1` |
| 6 | 16 | device id |
| 22 | 8 | sequence number |
| 30 | 1 | kind: `0` = op, `1` = snapshot |
| 31 | 24 | nonce |

The header is **authenticated but not encrypted**. Device id and sequence are
already visible in the path, so they are not secret; binding them into the AEAD
means a chunk that has been moved, renamed or duplicated inside the folder
fails to open rather than being silently accepted somewhere it does not belong.
The kind byte is bound for the same reason: renaming `.op` to `.snap` must not
make a reader treat a delta as a whole document.

---

## 5. Encryption

Ciphertext is **XChaCha20-Poly1305** over the payload, with the 55-byte header
as associated data.

> ⚠️ The primitives come from the RustCrypto project and are not implemented
> here. The way they are assembled has **not** been independently audited.

### 5.1 Nonce derivation

```
nonce = device_id (16 bytes) || seq (8 bytes, little-endian)
```

Exactly 24 bytes, unique by construction. No RNG, no nonce database, no
possibility of a collision between devices.

### 5.2 What this depends on

Nonce reuse with *different plaintext* does not weaken XChaCha20-Poly1305, it
breaks it. This scheme is therefore safe **only** while a given `(device, seq)`
is never used for two different payloads. Two rules enforce that:

1. The sequence counter is flushed to disk before the chunk is written, so a
   crash burns a number instead of reissuing it. This is why gaps are legal —
   they are the cost of never reusing one.
2. Writing a chunk that already exists is refused rather than overwritten.

Any implementation that allocates sequence numbers by scanning the directory
for the highest one is unsafe: crash after writing the temporary file and
before the rename, and the next scan hands the same number to different
content.

### 5.3 Keys

The vault key is 32 random bytes. It is stored **outside the vault**, in the
per-user configuration directory:

| platform | location |
|---|---|
| Windows | `%APPDATA%\norm_note\keys\<vault-id>.key` |
| macOS | `~/Library/Application Support/norm_note/keys/<vault-id>.key` |
| other | `$XDG_CONFIG_HOME/norm_note/keys/` or `~/.config/norm_note/keys/` |

Outside, because plenty of people keep their whole vault in a synced folder. A
key stored in `.norm/` would be uploaded alongside the ciphertext it protects,
to the exact party the encryption exists to keep out.

The file contains the key as 64 lowercase hex characters and a newline. On Unix
it is created with mode `0600`. **It is not itself encrypted** — anyone who can
read your user profile can read it. Using the platform keychain belongs with the
GUI applications, where there is a session to unlock it.

### 5.4 Recovery phrase

The same 32 bytes, plus a 2-byte checksum (the first two bytes of
`blake3(key)`), encoded as **Crockford base32** in hyphenated groups of five:

```
1450P-30D1R-7H048-J2CA1-A5GQ3-0CHM6-RW3MF-1Y811-48HJ8-9964W-M0YK7
```

Alphabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ` — no `I`, `L`, `O` or `U`. On
input, `I` and `L` read as `1`, `O` as `0`, case is ignored, and any
non-alphanumeric character is skipped.

34 bytes need 55 characters, which carry 275 bits — three more than the payload
uses. **Those three trailing bits must be zero.** Without that rule the final
character can be mistyped into any of eight values and still decode to the same
key, which is a hole straight through the checksum.

This is deliberately *not* BIP39. Twenty-four words would be better on paper,
and it needs the standard's exact 2048-word list; an approximation would
produce phrases that look interoperable and are not.

---

## 6. Payload

The plaintext inside a chunk. All integers little-endian.

```
batch   := u16 version | u32 entry_count | entry*
entry   := u32 doc_len  | doc_utf8       | u32 change_count | change*
change  := u32 len      | bytes
```

- `version` is `1`.
- `doc_utf8` is the note's identity from §2.
- For a chunk of kind **op**, each `change` is one Automerge change.
- For a chunk of kind **snapshot**, each entry has exactly one `change`, which
  is a complete Automerge document (`Automerge::save()` output).

A decoder must reject trailing bytes, and must not allocate based on a declared
count without checking it against the bytes actually remaining.

One chunk carries **many notes**. One chunk per note would mean one durable
write per note — thousands of them to import an existing vault.

---

## 7. Replication

Bringing two stores into agreement is a file copy: for every chunk present in
the source and absent in the destination, copy the bytes.

There is nothing to merge, nothing to order, and no winner to pick. Running it
twice, half way, or in either direction converges to the same result. **The
relay never needs the key** — it moves ciphertext.

Sequence counters are *not* copied. A device allocates sequence numbers only
for itself; advancing the local counter to match replicated chunks would make
it start issuing numbers another device has already used under the same key.

---

## 8. Snapshots and pruning

Every save appends, so the log grows without limit. A **snapshot** chunk
carries the writer's complete state for the notes it names, and supersedes that
writer's earlier chunks.

Reading a device's log means: apply its highest-numbered snapshot, then apply
its chunks with a higher sequence number than that snapshot.

Applying a snapshot **merges** it; it does not replace. A device receiving a
snapshot may hold edits its author never saw.

### 8.1 Pruning

Chunks from device `D` with `seq < S` may be deleted once a snapshot from `D`
at sequence `S` exists — **and only once that snapshot is also present in the
sync target**. Otherwise a machine that has not synced in months returns to
find the history it needed deleted and the replacement never sent.

**Every holder of the snapshot prunes, not just the device that wrote it.** The
first implementation had each device prune only its own chunks, on the
reasoning that another device's log was not its to judge. It does not work:
machine A deletes its old chunks from itself and the target, then machine B —
still holding copies — pushes them straight back on its next sync. Nothing is
ever removed. A snapshot *is* the judgement, and it came from the device whose
chunks it replaces, so any holder of it can act on it.

Pruning is never required for correctness, only for size. Skipping it costs
disk; doing it too eagerly costs notes.

---

## 9. Workspace state

Local only. Never replicated, never required.

### `.norm/config`

```
# comments allowed
vault_id = <32 hex>
device   = <32 hex>
target   = <path>      # absent for a local-only vault
```

No secrets belong in this file — `.norm/` may end up in a synced folder.

Device ids must differ between machines sharing a vault, or their chunks would
collide on the same paths.

### `.norm/state/materialized`

One line per note, recording what was last written to or read from its file:

```
<hash> <mtime-nanos> <size> <path>
```

`hash` is 64 hex characters (blake3 of the file text). `path` is the whole rest
of the line, since note names contain spaces.

This is how a user's edit is told apart from our own write — the two are
identical on disk, and the only difference is whether we put it there.

An unreadable `materialized` file is survivable: with nothing recorded every
file looks unfamiliar, and the ambiguity handling in §10 keeps both versions.

### `.norm/state/inflight`

Same format. Present **only** while a batch of note files is being written, and
therefore only after a crash.

Note files are written without `fsync` — they are derived from the log, and one
lost to a power cut can simply be written again. That is safe only because this
journal exists: on startup, any file listed here that does not match what was
intended is damage, not an edit, and is rewritten from the CRDT.

An unreadable `inflight` file is **not** survivable and must be an error. Ignoring
it means a torn file goes unrepaired and is then published to every device as
though the user had typed it.

### `.norm/rescued/<hash>.md`

A version that would otherwise have been discarded, kept verbatim. See §10.

---

## 10. Reconciling files with the log

One pass, in this order, always:

```
1. pull_from_disk    edits made outside the app become CRDT changes
2. sync              chunks out, chunks in, remote changes applied
3. push_to_disk      the merged result is written back
```

Reversing this destroys data. If a remote change has been absorbed but not yet
written to disk, reading the stale file and treating it as the truth deletes
that remote change.

For each file found on disk:

| recorded state | on-disk content | action |
|---|---|---|
| matches | — | nothing happened |
| differs | — | the user edited it; fold in |
| nothing recorded, note unknown | — | a new note |
| nothing recorded, note known and different | — | **ambiguous** |

The ambiguous case cannot be resolved: nothing says which version came first.
The file on disk is kept — it is the one the user can see — and the other
version is written to `.norm/rescued/`. Nothing is discarded.

### 10.1 Deletion

**A file that vanishes is written back.** There is no delete operation in
version 1.

A missing file could mean the user deleted the note, or moved the vault, or
restored a partial backup, or that a sync client is mid-operation. Guessing
wrong destroys notes on every device at once. Recreating an unwanted file
annoys someone; deleting a wanted one cannot be undone.

---

## 11. Reading a vault without this software

If you need your notes: they are the `.md` files. That is the whole answer.

If you need the history as well, the shortest path is
`crates/norm-core/src/oplog.rs` and `payload.rs`, which are together under 600
lines, and the `automerge` crate. You will need the vault key from §5.3.
