# Fixture provenance

`fixtures/artifact.html` is derived from `bench/Wiki Template -
Grove.html` (a bundler-packaged prototype export). The bundler wrapper
stores the app as a JSON-escaped `__bundler/template` block plus
gzip+base64 resources in a `__bundler/manifest`; that indirection is
noise for a conversion episode, so the fixture is the same app
reassembled standalone:

- the template HTML decoded from the `__bundler/template` block,
- each `<script src="{uuid}">` replaced by the inlined (gunzipped)
  manifest resource,
- font `url({uuid})` references inlined as `data:font/woff2` URIs.

Nothing was rewritten — markup, styles, and the `class Component`
logic are byte-identical to the export, just readable and runnable
from a single file. The harness screenshots this file directly as
`reference.png` (see `shots.sh`), so fixture and reference cannot
drift apart.

NOTES.md is harness-inert (only `fixtures/` is copied into the
episode site; the judge sees only rubric + shots + transcript).
