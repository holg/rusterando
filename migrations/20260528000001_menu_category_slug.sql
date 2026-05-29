-- SEO: URL slug for category pages (/menu/<slug>). Additive + nullable;
-- backfilled at server boot from seo_slug(name) with collision suffixes.
ALTER TABLE menu_categories ADD COLUMN slug TEXT;
