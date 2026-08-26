# Accessibility checklist — brain-client (WCAG 2.2 AA)

> v1.16.3 "Accessible". This is the **manual screen-reader pass** — the
> irreplaceable human gate. Automated scanners (axe) catch 20–60%; the rest
> requires a real screen reader. Re-audited per release; NOT an official
> VPAT/ACR.

## Coverage matrix

| Platform | Screen reader | Panels (Review/Recall/Subjects/Security/Audit/Health/Connect) | Status |
|---|---|---|---|
| macOS | VoiceOver | All | ☐ |
| Windows | NVDA | All | ☐ |
| Android | TalkBack | All (v1.16.4 mobile pass) | ☐ |

## How to run

```sh
dx serve --platform web   # then open the served URL
```

Tab through every panel with eyes closed + the platform screen reader on.

## Checklist per panel

- [ ] Navigate to the panel via keyboard alone.
- [ ] Focus moves to the `<h1>` on route change (SPA focus management).
- [ ] Document title updates on route change.
- [ ] All interactive elements are reachable via Tab.
- [ ] Button/link labels are descriptive (not "click here").
- [ ] Status changes (connection, chain verify) are announced.
- [ ] The context drawer traps focus while open; Esc closes it; focus returns
      to the trigger on close.
- [ ] No keyboard trap (can Tab out of any region).
- [ ] Form fields have associated labels.

## Code-level gates (automated)

These run in `cargo test` and are the automated subset of this checklist:

- `tests::interactive_elements_are_buttons` — no `<div onclick>` (WCAG 2.1.1
  Keyboard + ARIA in HTML). All clickable elements are real `<button>`s.
- `tests::xss_escape_hatch_is_unused` — no raw-HTML escape hatch (text is
  escaped by default).
- `PageTitle` (every panel) — the `<h1>` gets `tabindex="-1"` + focus-on-mount.
- `use_document_title` (every panel) — reactive document title per route.
- v1.28.39 "Access" WCAG 2.2 AA gate suite (`src/a11y.rs` + `src/i18n.rs`):
  `focus_never_obscured_by_docks` (2.4.11), `drag_alternatives_exist_for_
  every_drag` (2.5.7), `target_size_floor_24px_enforced_by_classes` (2.5.8),
  `help_entry_consistent_across_panels` (3.2.6),
  `no_redundant_entry_in_approval_flow` (3.3.7), `rtl_mirroring_smoke_all_
  panels` + `pseudolocale_elongation_renders_without_truncation` (G4),
  `acr_remarks_cover_every_non_support`, and the release-blocking
  `wcag_22_aa_gate_blocks_release`. Logical CSS properties (`ms-*`/`me-*`/
  `ps-*`/`pe-*`/`text-start`) replace physical direction utilities so the
  layout mirrors under `[dir="rtl"]` without duplicate rules.

## Known ceilings

- **axe-core browser gate is an operator/tooling step (v1.18.0)** — needs
  Playwright + a `dx bundle` + a live server + browser download; not runnable
  in this repo's CI surface. When runnable: serve the bundle, run axe against
  every route, fail on violations.
- Full Tab-cycling focus trap + return-focus-to-trigger in the context drawer
  is the v1.18.0 pass (the drawer currently has `role="dialog"`/`aria-modal` +
  Esc-close; the Radix-style Tab cycle and focus restoration are deferred).
- No aria-live regions for async updates beyond the `role="status"` banners
  (connection + re-verify) **and the run transcript (v1.28.20 Cockpit:
  `aria-live="polite"` on the transcript body — new nodes announce without
  stealing focus)**. Targeted announcements evaluated per-panel.
- The Cockpit cheat-sheet drawer (`?` on a run) ships `role="dialog"` +
  `aria-modal` + Esc/`?`-close; the full Tab-cycle trap + focus restoration
  remain the same deferred ceiling as the context drawer above.
- ~~No RTL locale~~ — SUPERSEDED (v1.28.39 "Access"): `ar` ships as the RTL
  locale, `en-XA` is pinned at test time via fluent-pseudo
  (`pseudolocale_elongation_renders_without_truncation`), and the full-panel
  mirroring smoke runs in CI (`rtl_mirroring_smoke_all_panels`). Deep
  bidirectional-text audit remains an open ceiling (see acr-vpat.md).
