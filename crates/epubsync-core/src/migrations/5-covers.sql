-- Migration 5: the library holds a thumbnail of each book's cover.
--
-- The row is the `Cover` enum. `state` names the variant, `image` holds
-- the JPEG bytes of `image`, and `detail` holds the text of `unreadable`
-- and `undecodable`. The `CHECK` lines make every other shape an error
-- from SQLite, so a row cannot hold an image and a fault at once, and a
-- fault cannot lose its text. A book with no row is `Unknown`: core has
-- not read its file yet, and the next open reads it.

CREATE TABLE covers (
    book_id INTEGER PRIMARY KEY REFERENCES books(id),
    state TEXT NOT NULL,
    image BLOB,
    detail TEXT,
    CHECK (state IN ('image', 'none', 'unreadable', 'undecodable')),
    CHECK ((state = 'image') = (image IS NOT NULL)),
    CHECK ((state IN ('unreadable', 'undecodable')) = (detail IS NOT NULL))
);
