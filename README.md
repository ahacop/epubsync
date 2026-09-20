# EpubSync

EpubSync manages a library of KEPUB files, edits their metadata, and syncs
them to a Kobo. Import converts every EPUB to KEPUB with kepubify, which is
compiled into the binary. The app does not read books and does not fetch
metadata from the internet.

## Install

With Nix:

```sh
nix run github:ahacop/epubsync -- --help
```

With Homebrew:

```sh
brew install ahacop/tap/epubsync
```

Nix builds the binaries from source with Rust and Go. Homebrew downloads
prebuilt Apple Silicon binaries from the GitHub Release for the tag. It
installs the `epubsync` command and the `epubsync-app` viewer. Each is
one file and needs no other program on `PATH`.

## Commands

```sh
epubsync init ~/Books/epubsync      # create the library folder and point the config at it
epubsync import book.epub           # convert to KEPUB and add it; a folder imports every EPUB in it
epubsync list                       # every book: id, title, authors, series, progress per device
epubsync list --reading --sort title # the books being read, in title order
epubsync show 3                     # one book's whole record, stats, file path, and progress
epubsync edit 3                     # open the metadata as TOML in $EDITOR
epubsync edit 3 --title "New Title" # set one field without the editor
epubsync remove 3                   # delete the file and its rows
epubsync sync                       # make the Kobo's EpubSync folder match the library
epubsync sync --dry-run             # print the plan and change nothing
epubsync eject                      # unmount the Kobo and end the USB session
epubsync words                      # words looked up on the Kobo, newest first
epubsync list --json                # the same data as JSON, for a script
```

The library is one folder. It holds every book as `<id>.kepub.epub` and the
database `library.sqlite`. Copy the folder to back it up. The config file
holds the folder path and lives in the XDG config directory.

`list` prints the books in id order. A word after `list` keeps only the books
with that text in the title, an author name, or the series name, as the
viewer's filter field does. `--title`, `--author`, and `--series` match one
field each. `--reading`, `--finished`, and `--unread` keep the books in that
state on the device they were read on last. `--sort` takes `id`, `title`,
`author`, `series`, `words`, `ease`, `progress`, `last-read`, or
`finished`. Several keys, as `--sort author,title`, break ties in turn, and
`--reverse` turns the whole order around. A title sorts without a leading
"The", "A", or "An", and a book with no value for a key comes last. These
are the rules the viewer's columns use.

Each sync that finds a book's progress changed adds a row to the book's
history, so `show` lists every read a sync saw, oldest first, with the day
it was seen. A book's finished date is the last read time of the sync that
first saw it finished. The date stays while the book is opened again and
moves when it is finished a second time.

`edit` flags: `--title`, `--publisher`, `--description`, `--author "Name|Sort"`
(repeat for several authors), `--series`, and `--series-number`.

`--json` on `list`, `show`, `words`, and `sync --dry-run` prints the same
data as JSON, so a script can read it without splitting the text lines.
`list` prints an array of books and `show` prints one book. A book is one
flat object: `id`, `revision`, `title`, `authors` with a name and a sort
name each, `series` with a name and a number, `publisher`, `description`,
`word_count`, `reading_ease`, `file`, and `progress` with one entry per
device: the serial, the percent, the status, the last read time, the
reading time in seconds as `time_spent`, and `finished_at`. `show` adds
`history`, the book's rows oldest first, each with `seen_at`, the time of
the sync that read it. A field the book does not have is left out. `words`
prints an array of words, newest first, each with the word, the device
serial, the book id and title, and the time it was looked up.
`sync --dry-run --json` prints the device, the write gate, and the actions,
each with its book title.

## Viewer

The viewer is a window that shows the library as a table with sortable
columns and a filter. A click on a column header sorts by that column, and
the filter field narrows the table by title, author, or series. A click on
a row opens the book's details in a sidebar: its title, authors, series,
publisher, description, reading progress per device with the finished date
and the reading time, the words looked up in it, and file path. The table's
Finished column holds the day a book was finished, and the status bar
counts the books finished this year. The Words tab in the toolbar swaps the
table for every word looked up on a device, newest first, with the book,
the device, and the day, and the filter field narrows it by word or book.
The Open button in the sidebar opens the book in the system reader, and a
link in a description opens in the browser. The Import button
and a drop of files onto the window add books. The Remove… button in the
sidebar removes the selected book after a dialog that says what the
removal does. The CLI stays the way to edit and sync.

```sh
nix run github:ahacop/epubsync#app   # or, after brew install: epubsync-app
```

The viewer holds the library while its window is open, so a CLI command
fails with "another EpubSync is running" until the window closes.

## Sync

`sync` finds the Kobo under `/run/media/$USER`, `/media`, `/media/$USER`, or
`/Volumes`, or takes `--device <path>`. It copies books the device lacks,
replaces books edited since the last send, and deletes device files whose
book left the library, after you confirm. Then it reads reading progress and
looked-up words back from the Kobo database.

A book deleted on the Kobo is sent again on the next sync. To take a book
off the device, remove it from the library.

Series and the author string show on the Kobo on the sync after the one that
sent the book, because the firmware creates the book's row when it imports
the file.

Every write to the Kobo database is gated on its `dbversion`. On a version
the app has not been run against, sync skips replacements and row updates
and says so. `--allow-newer-firmware` runs them anyway.

## Eject

`sync` leaves the Kobo mounted, so you can run more commands. When you are
done, `epubsync eject` unmounts it and sends the SCSI eject that ends the
USB session; after a plain unmount the Kobo keeps showing "connected".
Pull the cable only after the eject, or the Kobo database can be left
corrupt. After an eject the Kobo drops off the USB bus, and only a new
plug-in brings it back.

On Linux the eject goes through udisks2 over D-Bus, so udisks2 must be
running, and polkit decides whether your session may unmount and eject. A
logged-in local user may by default. On macOS it runs `diskutil eject`.

## Build

The dev shell has rustc, cargo, Go, and SQLite. With direnv, an `.envrc`
that holds `use flake` loads it whenever you enter the folder:

```sh
nix develop
cargo test
```

Go is a build-time dependency only. `crates/epubsync-core/build.rs` compiles
the shim in `kepub-shim/` with `go build -buildmode=c-archive` and links it
statically. The shim's dependencies are vendored, so no build needs the
network.

## Release

`just release 0.1.2` bumps the version, commits, tags, and pushes. The
release workflow builds the CLI for Apple Silicon and attaches the tarball to
a GitHub Release. The recipe then waits for that release and points the
Homebrew formula at the new tarball. It expects the tap checkout at
`../homebrew-tap`, or pass `TAP=<path>`.
