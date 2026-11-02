# Web UI rules

- Every page starts with `PageHeader` (`src/components/page-header.tsx`): back link, title, facts in `meta`, buttons in `actions`. Don't build custom title blocks.
- No taglines, subtitles, helper paragraphs or secondary labels. If a screen needs one to make sense, change the layout: put the fact in `meta`, a `KeyValue`, a heading or a `StatusBadge`.
- Text is allowed only for state the user must act on: errors, empty states, permission gaps, and irreversible-action dialogs (one sentence in the `Modal` description).
- Show a fact once. Facts in the header or a table row are not repeated in detail grids below.
- Hide what is healthy. Show health, summaries and hints only when they differ from the default (see `SiteHeader`, `AdvancedSettings summary`).
- Reuse `status.tsx` primitives (`Field`, `KeyValue`, `StatusBadge`, `Empty`, `ErrorNotice`, `Loading`) and shadcn `ui/` components instead of raw `<label>`/`<select>`.
- Name files after the component they export (`metric-strip.tsx` exports `MetricStrip`).
- Tests find elements by role and accessible name. Keep those names stable, and don't assert on explanatory copy.
