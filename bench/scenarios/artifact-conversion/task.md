You are working in a directory that is a slide site, already connected
to its remote. The file `artifact.html` in this directory is a web app
a user built as a Claude artifact. The user wants it converted into
the tonk system so the data lives as concepts and the UI renders as
tonk views, editable and syncable like any other tonk data.

Use the slide CLI (run `slide guide` to learn it). Convert the
artifact:

1. Model the artifact's data as one or more concepts. Name the main
   concept `item`.
2. Assert the artifact's current data (all five items, with their
   packed state and category).
3. Recreate the UI as a tonk view named `packing-list` over that data,
   matching the artifact's content and intent as closely as the view
   system allows: item names, categories, packed/unpacked distinction,
   and the packed count if possible.

Stop when `slide status` reports the branch is synced.
