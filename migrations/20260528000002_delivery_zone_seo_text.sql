-- SEO: optional admin-editable marketing copy for the per-area delivery
-- landing page (/lieferservice/<slug>). NULL/empty → the page renders a
-- generated German fallback paragraph instead.
ALTER TABLE delivery_zones ADD COLUMN seo_text TEXT;
