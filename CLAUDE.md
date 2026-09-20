# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Toolchain

`cargo` and `go` come from the Nix dev shell. An `.envrc` in the repo root
that holds `use flake` loads that shell through direnv, so the tools are on
`PATH` and the commands below run as written. Without direnv, prefix each
command with `nix develop -c`, or enter `nix develop` first. The `just`
recipes do this on their own.

The first build compiles the Go shim in `kepub-shim/` with
`go build -buildmode=c-archive` and links it statically. The shim's
dependencies are vendored, so the build needs no network. Go is a build
dependency only: CI fails if the binary links a Go or kepubify shared
library.

## Commands

```sh
cargo test --locked                        # what CI runs
cargo test -p epubsync-core --test kobo_db # one test file
cargo test -p epubsync-epub splice::       # unit tests by module path
cargo test -p epubsync-cli syncs_to        # tests by name substring
cargo clippy --workspace
cargo fmt
just cli list                              # run the CLI from the tree
just app                                   # run the viewer
nix build && nix build .#app               # the release packages
just release 0.1.8                         # bump, tag, push, update the tap
```

Integration tests live in each crate's `tests/`. The `epubsync-epub` unit
tests live next to the code in `src/<module>/tests.rs`.

## Crates

The workspace has four crates in one dependency direction:

- `epubsync-epub` reads and writes EPUB files. `Epub` is the whole
  interface. Nothing above it sees the zip, the OPF, or the XML.
- `epubsync-core` holds the library, the device layer, the sync, the Kobo
  device, the config, and the kepubify FFI.
- `epubsync-cli` (binary `epubsync`) and `epubsync-app` (binary
  `epubsync-app`) sit on top of core. The viewer imports and removes
  books; every other write goes through the CLI.

### epubsync-epub

`opf.rs` parses the OPF and records the byte range of each element the app
owns (title, creators, series, publisher, description, word count, reading
ease). `splice.rs` writes a record back by replacing only those ranges.
`archive.rs` rebuilds the zip with the OPF entry replaced and copies every
other entry raw. Together they make an edit keep every byte the app does
not own. The crate's doc comment lists the publisher quirks the parser
absorbs. Add a new quirk there when you handle one.

The `fixtures` feature exposes `fixtures.rs`, which builds EPUBs in memory
from an OPF string. Other crates' tests import it as
`epubsync_epub::fixtures`. The CLI tests build their own zip because they
run the binary.

### epubsync-core

A library is one folder: `<id>.kepub.epub` per book, `library.sqlite`, and a
`lock` file. `Library::open` takes an exclusive file lock and holds it until
the `Library` drops, so a command and the viewer never run at once. It then
runs the migrations in `src/migrations/*.sql` through `rusqlite_migration`,
which tracks them with `PRAGMA user_version`. A schema change is a new
numbered file. Never edit an existing one.

Each book row has a `revision`. `edit` adds 1 to it and then writes the
record into the file. The `sent` table holds the revision last sent per
book per device. `device::plan` is pure: it takes the book revisions, the
ids on the device, and the `sent` rows, and returns the actions. `sync.rs`
runs plan, gate, apply, row update, and read back in that order, and
updates `sent` after each book so an interrupted sync resumes.

Progress read back from a device goes into `progress_history`, one row
per change a sync sees, and the `progress` view selects the newest row
per book per device. A history row's `finished_at` is the `last_read` of
the first row that reads as finished, copied to every later row and
replaced when the status turns finished again. `sync::next_row` is the
pure rule that decides whether a read adds a row and what its
`finished_at` is.

`cover.rs` holds the `Cover` enum and `thumbnail`, which decodes a cover
and re-encodes it as a JPEG that fits in 480 by 480 pixels. The `covers`
table holds one row per book: `state` names the variant, `image` holds
the thumbnail bytes, and `detail` holds the text of a fault. `CHECK`
lines make every other shape an error. A book with no row is
`Cover::Unknown`, and `Library::fill_covers`, which `Library::open`
calls, reads the file of each active book that has none. `Library::cover`
reads one row and opens no zip, so the viewer calls it while it draws.

`Device` is the trait. `Kobo` is the only implementation: a mounted volume
with `.kobo/version`, books under `EpubSync/`, and the database at
`.kobo/KoboReader.sqlite` opened in place. The column names and the file
size rule come from Calibre's Kobo driver. Every write is gated on
`dbversion`. A version not in `TESTED_VERSIONS` makes sync skip
replacements and row updates.

`query.rs` holds the filter and sort rules. `Query::select` takes the
books and the progress rows and returns the books to show, in order. The
`list` flags and the viewer's filter field and column clicks both build a
`Query`, so a change to a rule lands in both.

`kepub.rs` calls `KepubConvert` from the Go archive. `build.rs` compiles
that archive, and the crate declares `links = "kepubshim"`.

`config.rs` reads one key, the library path, from the XDG config file or
from the path in `EPUBSYNC_CONFIG`. Tests set that variable to a temp file.

### Tests that fake a Kobo

Core tests make a Kobo from a temp folder with a `.kobo/version` file. The
`kobo_db` tests also build the Kobo database from
`tests/fixtures/kobo-schema.sql` and insert a `dbversion` row. The CLI tests
use `assert_cmd` against the built binary with a temp library.

### epubsync-app

An Iced 0.14 window. The state is the `Viewer` enum in `main.rs`. It
holds the open `Library`, what the panes draw, and the import under way.
The window follows the system light or dark mode: `theme.rs` style
functions read `is_dark` from the theme at draw time, so no view function
knows the mode. The fonts are embedded from `fonts/`.

`Open.covers` holds a `Handle` per book the sidebar has shown, so a
second click on a book reads no row. `select` fills the entry through
`Library::cover`. A book the library has no cover for gets `None` and the
words "No cover"; a book core has not read yet gets no entry, and the
next click asks again.

`import.rs` runs `Library::import` per file on a background task. Iced
messages must be `Clone`, so the task takes the `Library` value and hands
it back inside `Handoff`, and the state holds `None` in between. Reload is
off while the library is away. A write from the viewer ends with a reload.
While the import strip is shown, the books pane draws the rows of the
strip's tab (Added, Skipped, Failed) through the same query, and `Open`
keeps the query and the scroll offset from before the import in `before`
until the × puts them back.

`remove.rs` draws the remove dialog over the window with `stack` and
`opaque`, the Iced modal pattern. `Open.removing` holds the book id while
the dialog is shown, and `Message::Close` (Escape) cancels the dialog
before it closes the sidebar. The doc comment in `remove.rs` lists the
dialog conventions: the title names the action and the book, the body
says what happens, the buttons are verbs, Cancel sits left of the red
Remove, and Enter does nothing.

`full_cover.rs` draws the full-size cover over the window with the same
`stack` and `opaque` pattern. A click on the sidebar thumbnail sends
`Message::ShowCover`, which calls `Library::full_cover` to read the image
out of the book file. `Open.full_cover` holds the handle while the cover
is shown, and `Message::Close` (Escape) closes it before it closes the
sidebar.

The Open button and the description links go through the `opener` crate
on a background task, because on macOS `open` waits for the command to
exit. A failure lands in `Open.error`, which the status bar shows.

## Versions and releases

The CLI's `build.rs` sets the printed version from `git describe`, so a
working tree build reads like `0.1.7-3-gabc1234-dirty`. A build without
`.git` prints the Cargo version. The release recipe bumps `Cargo.toml` and
`flake.nix`, tags, pushes, waits for the macOS release workflow, and then
points the Homebrew formula in `../homebrew-tap` at the tarball.

`CHANGELOG.md` gets a prose entry per release before the release commit.
