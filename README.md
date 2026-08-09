# Norm Note

A minimalist sanctuary for your thoughts. A local-first Markdown vault built on Rust for unprecedented speed and focus.

No LLM. No telemetry. No server. No subscriptions.

> **Status: v1.0.3 Release.** Norm Note has evolved from a CLI sync daemon into a full-featured desktop application with a beautiful, minimalist UI. 

---

## What is Norm Note?

Norm Note is a desktop application that keeps a plain Markdown vault in sync through **a folder you already own**: Dropbox, iCloud Drive, a NAS mount, or an external disk.

- **Write at the speed of thought:** Real-time Markdown rendering. Headings, bold text, lists, and images style themselves seamlessly without switching between 'edit' and 'preview' modes.
- **Infinite Organization:** Create infinite nested folders, drag and drop notes instantly, and manage thousands of projects without losing track of a single file.
- **Fast Search:** Find anything in milliseconds. Press `Cmd+K` and jump straight to the exact line in your notes.
- **No Vendor Lock-in:** Your notes are saved as standard `.md` files. You own them forever. Export to PDF, HTML, or plain text with a single click.

## The three guarantees

**G1 — Zero Egress.** This software never connects to any host we control. No telemetry, no crash reporting, no analytics, no update check, no online licence validation, no fonts from a CDN, no model API. This is enforced by the dependency graph in [`deny.toml`](deny.toml) and checked in CI.

**G2 — Outlive the vendor.** The on-disk format is Markdown and YAML. The core is open source. If this project stops shipping, your vault keeps working perfectly.

**G3 — It's just files.** Delete the app and every note still opens in any standard text editor. Edit a note in vim and it merges correctly.

## Cryptography

⚠️ **Not independently audited.** Synchronization chunks are encrypted with XChaCha20-Poly1305 using implementations from the RustCrypto project. The way those pieces are assembled has not been reviewed by an external expert.

## Architecture & Layout

Norm Note is built with a Rust backend (Tauri) and a high-performance frontend for a seamless native experience.

```text
crates/norm-core      vault, oplog, CRDT, reconciliation
crates/normd          daemon and CLI core logic
norm_ui_vault         frontend UI components and styling
website               landing page and documentation
docs/FORMAT.md        the on-disk and sync specification
```

## Building

```bash
# Build the core workspace
cargo build --release

# Run tests
cargo test --workspace
cargo deny check bans licenses advisories
```

## Licence

Core: MPL-2.0.
