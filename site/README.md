# Site overlays

What one deployment adds that is that shop's own and so does not belong in
the public tree. One folder per deploy profile (`-e <profile>` in
`scripts/deploy_to_server.sh`): `site/<profile>/`. Everything under `site/`
except this README and `site/example/` is gitignored.

```
site/<profile>/
  posters.typ                  poster layouts: live prices on the shop's cover artwork
  apple-app-site-association   the shop's AASA manifest (Team ID + bundle id)
  public/                      laid over html/ on deploy (e.g. img/ladenfront.jpg)
  seeds/*.sql                  applied by `apply-seeds` (after data/*.<profile>.sql)
  tests/poster.jpg             one poster picture + tests/poster-expect.txt for the tests
  tools/                       the shop's own scripts (poster builds, ops helpers)
```

All parts are optional.

## How it is used

- **Deploy:** `cmd_upload` ships `site/<profile>/` to `$REMOTE_BASE/site/<profile>/`
  and rsyncs its `public/` over `html/`. Backups and restores include `site/`.
- **Server:** set `SITE_OVERLAY_DIR=./site/<profile>` in `.env.<profile>`. The
  server reads it once at startup:
  - `posters.typ` is served to the PDF templates as `/site_posters.typ`
    (see `templates/cover_live.typ`); without it, no poster layouts.
  - `apple-app-site-association` replaces the placeholder at
    `/.well-known/apple-app-site-association` (only if it is valid JSON).
- **Placeholders:** `public/img/{cover,ladenfront,pizza-1,pizzablech,theke}.jpg`
  are neutral pictures; an overlay's `public/img/` replaces them on that site.
- **Tests:** `SITE_OVERLAY_DIR=$PWD/site/<profile> cargo test -p rusterando-server`
  runs the poster tests against that overlay; without it they are skipped.

`site/example/` shows the format.
