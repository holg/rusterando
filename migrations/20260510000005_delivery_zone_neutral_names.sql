-- Make delivery-zone display names brand-neutral.
--
-- Migration 20260508000005 set the names to "Lüdinghausen / Seppenrade /
-- Bauerschaften" — three Davids-specific Stadtteile that any fork would
-- ship verbatim. We can't edit the original migration (sqlx checksums)
-- so this one resets the names back to placeholders.
--
-- Per-deployment reality lives in data/branding.davids.sql (or whatever
-- shop-specific seed the deploy script applies after migrations). Each
-- shop's seed UPDATEs the names back to its own real Stadtteile.
--
-- Idempotent + safe on Davids' prod: the WHERE clauses only match the
-- old default names, so a deploy that's already customised the zones
-- via /admin (or via a real-name seed) is unaffected.

UPDATE delivery_zones SET name = 'Stadt'
    WHERE id = 'dz-luedinghausen' AND name = 'Lüdinghausen';

UPDATE delivery_zones SET name = 'Vorort 1'
    WHERE id = 'dz-seppenrade'    AND name = 'Seppenrade';

UPDATE delivery_zones SET name = 'Vorort 2'
    WHERE id = 'dz-bauerschaften' AND name = 'Bauerschaften';
