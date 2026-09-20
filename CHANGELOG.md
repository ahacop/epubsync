# Changelog

## Unreleased

The library keeps a reading history. Sync adds a row to a new
`progress_history` table each time a book's percent, status, last read
time, or reading time differs from what the last sync read, and a
`progress` view over the table holds the current state, so every command
that read `progress` still works. Migration 4 copies the old rows into the
history. A row records the Kobo's reading time in seconds and the date the
book was finished, which is the last read time of the sync that first sees
the status as finished. The date stays with the book while it is opened
again and moves when it is finished a second time. `remove` keeps the
history rows.

The viewer's table gets a Finished column after Last read. The sidebar's
device block says "finished 12 May" in place of "read 12 May" for a
finished book and adds the reading time as "3 h 20 min of reading". The
status bar counts the books finished this year. `list --sort finished`
orders by the finished date. `list` and `show` print the finished date
and the reading time per device, `show` prints a History line per row,
and `show --json` adds a `history` array. The progress objects in `--json`
output gain `time_spent` and `finished_at`. The sync report says how many
books' progress changed.

The viewer gets a Reload button at the right of the toolbar. It reads the
books, the progress, and the words again and keeps the sort, the filter,
and the sidebar. The `Library` value now lives in the viewer state, which
is what the writes from the viewer will need.

The viewer imports books. An Import button opens the system file picker
on EPUB files, and files dropped onto the window import too, as does a
folder, one level deep as with `epubsync import`. Each file goes through
the same import as the CLI on a background task, so the window stays
live while kepubify runs. A strip under the toolbar shows the file in
flight, a progress bar, and a Cancel button, which drops the queued
files once the file in flight finishes. Under that line the strip has
three tabs, Added, Skipped, and Failed, with their counts, and the table
shows the rows of the tab in view in place of the library: the added
books as they land, the library book each skipped file matched, or the
failed files with their errors. A skipped file is one whose title and
first author are already in the library. When the import ends, the
strip reads "Imported 12 files in 1 min 14 s", and the × on it puts the
table back to the sort, the filter, and the scroll from before the
import.

The viewer removes books. The sidebar gets a red Remove… button, and
a dialog over the window asks "Remove “The Left Hand of Darkness”?" and
says what happens: the file is deleted from the library folder, the next
sync takes the book off any device it was sent to, and the reading
history and the looked-up words stay. Cancel sits left of a red Remove
button, and Escape or a click outside the dialog cancels. The removal is
the same as `epubsync remove`, and the table reads the library again
after it.

The viewer opens books. An Open button under the title in the sidebar
opens the book in the system reader. A link
in a description opens in the browser; before, a link click did nothing.
Should the reader or the browser fail to start, the status bar says why.

An import that fails after the conversion, and an edit whose zip rewrite
fails partway, no longer leave a temp file in the library folder. Both
now build in a temp file that is removed when the step returns an
error. The import temp file is named `import-<random>.tmp` in place of
`import.tmp`.

## 0.1.8 (2026-09-17)

A sync from macOS no longer leaves a `._` file next to each book it sends.
The copy to the Kobo now writes only the book's bytes, so macOS has no
extended attributes to store on the FAT volume. Sync also deletes the `._`
files that earlier syncs left in the `EpubSync` folder.

## 0.1.7 (2026-09-14)

A library made by 0.1.6 does not open with this release. The schema is now
one migration, and a database from 0.1.6 has the tables but no version
stamp, so the migration fails on the first table. Run `epubsync init` on a
new folder and import the books again.

The viewer shows each book's word count and Flesch reading ease. The table
has a Words column and an Ease column between Series and Progress, both
sortable, and the sidebar shows a Length line and an Ease line with the
Flesch band name. Import reads both numbers from a file that carries them,
as Standard Ebooks files do, and measures them from the text of a file that
does not. The measured numbers are written into the library file as the
same `schema:wordCount` and `schema:educationalLevel` elements. A book in a
language the scorer has no coefficients for gets a word count and no
reading ease.

The command and the viewer now open a book whose OPF puts a prefix such as
`ns0:` on a creator's attributes without declaring it. A write of such a
file declares the prefix, so the file becomes well-formed XML. A chapter
file whose name holds a space or another percent-escaped character is found
in the zip, so the spine walk and the cover lookup no longer stop at it. A
chapter that starts with an XML declaration parses.

A build from the working tree prints the git description as its version,
such as `0.1.6-16-g7a09b62-dirty`, so a dev build is told apart from the
release.

`just cli <args>` runs the command from the working tree, and a bare `just`
lists the recipes.

Full changelog: https://github.com/ahacop/epubsync/compare/v0.1.6...v0.1.7

## 0.1.6 (2026-09-14)

The Homebrew package now includes the viewer. `brew install ahacop/tap/epubsync`
installs two programs: the `epubsync` command and the `epubsync-app` viewer.
Before this release the package held only the command, and a Mac user had to
build the viewer from source with Rust and Go.

The viewer is a window that shows the library as a sortable table with a
filter. A click on a row opens the book's details in a sidebar. It is
read-only. Start it from a terminal with `epubsync-app`. It ships as a plain
binary, not an app bundle, so it does not appear in Launchpad or Spotlight.

The release tarball for Apple Silicon now contains both binaries. Each one
needs no other program on `PATH`.

Full changelog: https://github.com/ahacop/epubsync/compare/v0.1.5...v0.1.6
