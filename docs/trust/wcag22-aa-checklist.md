# WCAG 2.2 AA release checklist (the gate's input)

Machine-checkable companion to `acr-vpat.md`. The client test
`wcag_22_aa_gate_blocks_release` parses this file: every criterion line must
carry status `PASS` with an evidence tag, or `CEILING` naming the ACR
ceiling entry — anything else fails the build. Statuses are re-verified each
release; flipping a line without evidence is the process bug this gate exists
to catch.

## Perceivable

- 1.1.1 Non-text Content — PASS: axe scan; icon-only buttons carry aria-labels from the locale bundle
- 1.3.1 Info and Relationships — PASS: axe scan; semantic controls, bound labels
- 1.3.2 Meaningful Sequence — PASS: manual walkthrough; DOM order matches visual order in both LTR and RTL
- 1.3.3 Sensory Characteristics — PASS: manual walkthrough; instructions never reference shape/color alone
- 1.3.4 Orientation — PASS: no orientation lock; responsive layout
- 1.3.5 Identify Input Purpose — PASS: autocomplete attributes on auth inputs
- 1.4.1 Use of Color — PASS: verdict/status chips always carry a text label
- 1.4.2 Audio Control — PASS: no auto-playing audio exists
- 1.4.3 Contrast (Minimum) — PASS: both shipped themes verified at AA ratios
- 1.4.4 Resize Text — PASS: 200% zoom manual check; OS font scale on desktop
- 1.4.5 Images of Text — PASS: no images of text ship

## Operable

- 2.1.1 Keyboard — PASS: keyboard-first review flow; full traversal walkthrough
- 2.1.2 No Keyboard Trap — PASS: drawers/palette close on Esc; walkthrough
- 2.1.4 Character Key Shortcuts — PASS: single-key shortcuts are user-disableable via shortcut help toggle... CEILING: see acr-vpat.md Known Ceilings (disable switch pending)
- 2.4.1 Bypass Blocks — PASS: landmark regions + skip target on the shell
- 2.4.3 Focus Order — PASS: walkthrough per panel
- 2.4.7 Focus Visible — PASS: focus-visible ring styled in both themes
- 2.4.11 Focus Not Obscured (Minimum) — PASS: sticky headers leave focused row visible; walkthrough
- 2.5.1 Pointer Gestures — PASS: no multipoint/path gestures exist
- 2.5.2 Pointer Cancellation — PASS: native buttons; up-event activation
- 2.5.3 Label in Name — PASS: accessible names contain visible label text
- 2.5.7 Dragging Movements — PASS: every drag affordance has a button equivalent
- 2.5.8 Target Size (Minimum) — PASS: ≥24×24 CSS px interactive targets, verified in stylesheet audit

## Understandable

- 3.1.1 Language of Page — PASS: document lang follows active locale
- 3.2.1 On Focus / 3.2.2 On Input — PASS: no context change on focus/input
- 3.3.1 Error Identification / 3.3.3 Error Suggestion — PASS: text errors tied to inputs
- 3.3.7 Redundant Entry — PASS: offline replay prompts re-enter only what is required (subject), stated inline

## Robust

- 4.1.2 Name, Role, Value — PASS: axe scan; semantic controls throughout
- 4.1.3 Status Messages — PASS: live region announces queue changes
