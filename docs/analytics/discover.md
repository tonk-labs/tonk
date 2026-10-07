# Discover popularity and returns

Successful `space_conversion` events with `conversion=created` now carry an
optional `template_id`. It is the first 16 hex characters of SHA-256 of the exact
catalog reference (`catalog URL#slug`), using the same anonymization as `space_id`.
The worker sends this event only after the template has been applied successfully.
Blank, seeded, duplicated, and joined spaces do not receive template attribution.
Catalog updates at the same URL and slug retain the same ID; URL or slug changes
start a new identity. No raw catalog URL, template title, space name, or content is
sent. To label reports, hash public references from the Discover catalog using
`tonk_analytics::anonymize` and maintain that mapping in the report. For example,
with a downloaded catalog (the URL must be the one the template was copied from: the
deployment's `TEMPLATE_CATALOG_URL` in `wrangler.toml`, served at
`/.well-known/tonk/discover`, or a catalog the account owner added in Discover):

```python
import hashlib
import json

catalog_url = "https://goblinoats.github.io/honky-tonks/catalog.json"
with open("catalog.json") as source:
    for template in json.load(source)["templates"]:
        reference = f"{catalog_url}#{template['slug']}"
        template_id = hashlib.sha256(reference.encode()).hexdigest()[:16]
        print(template_id, template["slug"])
```

`space_entered` carries `schema_version=1` and hashed `space_id`. The shell records
initial deep links and navigation into a different space, including returns via
the Hub. Changing routes within the current space does not emit another entry.
Reloading or opening another tab does. Entries represent navigation intent, not
successful rendering, editing, dwell time, or proof that a person used the app.

Join entries to successful creations by `space_id` to recover template attribution
across reloads and devices without introducing analytics storage in the product.
Use the creation event as the attribution table, deduplicated by `space_id`.

Recommended report definitions (filter `environment=production`):

- **Adoption:** distinct created `space_id` values by `template_id`. Also show
  distinct identified creator profiles, so repeated copying is visible.
- **Reach:** distinct profiles entering attributed spaces, grouped by template.
  This includes collaborators; separate the creator by comparing entry and
  creation profile IDs when measuring creator retention.
- **Usage frequency:** distinct `(profile, space_id, UTC date)` entry tuples.
  This prevents reloads, tabs, and repeated same-day visits inflating return counts.
- **Next-day / seven-day creator retention:** each profile enters a template's
  cohort once, at its first observed identified creation. Among creators with a
  full observation window, measure the fraction entering that same first space
  on UTC day 1 / any UTC day 1–7 after creation. Exclude day-zero entries,
  including the automatic navigation after creation. D1 matures at the start
  of UTC day 2; days 1–7 mature at the start of UTC day 8. Show eligible cohort
  sizes alongside rates. Later copies do not create additional cohort members.
- **Tonk retention by template:** a separate report counting later-day activity
  anywhere in Tonk for those creators. It answers whether a template brings users
  back to Tonk, rather than back to that particular space.

For profile reports require `identity_state=profile` on both creation and entry;
profiles are not accounts or unique humans. Early deep links can occur before
identity resolution and are excluded from those reports. Missing or opted-out
creation events leave entries unattributed. Existing spaces, duplicates, and
pre-instrumentation history cannot be retrospectively attributed. These are
observational cohorts, not evidence that a template caused retention.

## Dashboard maintenance

Published: [Discover templates](https://eu.posthog.com/project/70116/dashboard/989095).

`discover-dashboard.json` versions five SQL insights, the public template label
mapping, and the published dashboard IDs. Activity uses a fixed rolling 30-day
window; return cohorts use first creations from the last 90 days. Attribution
looks up creations across retained event history, so an older space can still
contribute to current activity. Unknown template IDs remain visible with an
"Unlisted template" label; update the mapping when the public catalog changes.
The queries use their own windows, rather than dashboard date overrides.

```sh
python3 scripts/posthog-discover-dashboard.py validate
python3 scripts/test-discover-dashboard.py
python3 scripts/posthog-discover-dashboard.py publish
```

The fixture executes the saved SQL against inline synthetic rows and never
inserts events. It covers repeated copies, duplicate events, collaborators,
unresolved identity, same-day reloads, return-window boundaries, immature cohorts,
unknown templates, and unattributed entries. Publishing checks the active project,
preserves other insight memberships, and verifies saved queries and tile order.

After deployment, create a Discover space and verify its `template_id` and
`space_id` reach PostHog, then confirm a subsequent entry joins to that space.
Full browser delivery and production ingestion remain separate checks. Early
blank retention values are intentional, not failed queries.
