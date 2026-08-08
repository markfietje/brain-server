# Store Readiness — brain (v1.17.0 M3)

What an App Store / Google Play submission needs, and the honest answers for
this client. The claim across every section is "**no data collected**" — and it
is accurate because the client stores only an auth token to a **self-hosted**
backend (the operator's own brain-server) and ships **no** analytics, no
tracking SDKs, and no third-party SDKs.

## Privacy nutrition labels

| Question | App Store answer | Play answer |
|---|---|---|
| Data collected | **None** | **No data collected** |
| Analytics | None | No |
| Advertising | None | No |
| Location | None | No |
| Contacts / photos / camera / mic | None | No |
| Third-party SDKs | None | No |
| User-generated content | None (client is a read/approve console, not UGC) | No |

Why it holds: the client talks to exactly one host — the operator-configured
backend URL (the base the Connect screen resolves). The only credential is the
auth token (in-memory on web; OS Keychain/Keystore on native, v1.16.6 M2). It
never reads the address book, never polls sensors, never runs in the
background (foreground-active only), and sends nothing to any host except the
configured backend. `verification 6` in `IMPLEMENTATION_PLAN_v1.17.0_Mobile.md`
(a network capture during onboarding) asserts this.

## App icons + launch screens

- Icons per Apple (App Store) and Google (Play) asset spec.
- Launch screen shows the brand mark on the theme background (`--color-background`).
- Screenshots front-and-center the **DSAR certificate with the green chain-verify
  badge** (the governance story), plus Review, Recall, and the offline Connect
  pre-fill.

## Deep links (foundation)

- `Dioxus.toml` registers the `brain://` custom scheme on iOS (`url_schemes`)
  and Android (`[[android.intent_filters]]`), so a `brain://…` link opens the
  app and the existing `Routable` router resolves it to the right panel.
- Full URL-parity (https universal links / app-links that mirror
  `/review/:id`, `/subjects/certificate/:id`, `/recall/:trace_id`) is the
  v1.19.0 milestone.

## Submission checklist

- [x] Privacy labels written (this file) — "no data collected", accurate.
- [ ] App icons + launch screens generated from the brand asset per platform spec.
- [ ] Screenshots captured from the live `/app` (DSAR-green-chain first).
- [x] Deep-link intent filters in `Dioxus.toml`.
- [x] Offline graceful — Connect pre-fills the last URL + shows the failure.

The icon/screenshot generation and the store upload itself are **operator
steps** (need the platform tooling + an Apple/Google developer account); this
file is the reproducible spec those steps execute against.
