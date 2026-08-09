# Reverse-Proxy SSO (B1) — Enterprise identity in front of Brain Server

> Enterprise plan §33.2 Phase B1 / §33.3 item 1. The cheapest enterprise
> door-opener: put an identity edge in front of brain-server so users sign in
> with their corporate account (Entra ID / Okta / Keycloak / Auth0) and every
> request to the server arrives authenticated.

## Why proxy SSO and not native SSO

Brain Server authenticates in two ways today (verified in code, Round 26):

1. **Opaque bearer mode** (default): `AUTH_TOKEN` / `AUTH_TOKEN_FILE`, constant-
   time compare, hot rotation.
2. **JWT mode** (opt-in): RS256/ES256/EdDSA verification against a **local**
   JWKS (PEM files in `BRAIN_JWT_KEY_DIR`), `(jti, iss)` revocation, refresh
   reuse detection.

The server is a token **validator**, not an OIDC **relying party**: there is no
login redirect, no PKCE exchange, no external JWKS fetch, no SAML, no SCIM.
Native OIDC RP is the 100% answer and is queued as **v1.20 B2**. Proxy SSO is
the 80% answer **shipped now, no server code changes**: an identity-aware
reverse proxy terminates the IdP login and forwards authenticated requests.

For SAML-shy orgs, Authentik / Keycloak bridge SAML → OIDC at the proxy, so
proxy SSO also covers SAML without building it into the server.

## Architecture

```
┌────────┐   ┌───────────────┐   ┌───────────────┐   ┌──────────────┐
│ User   │──▶│ SSO Proxy     │──▶│ Brain Server  │   │ IdP          │
│ browser│   │ OAuth2-Proxy  │   │ 127.0.0.1     │   │ Entra/Okta/  │
│ / curl │   │ / Caddy       │   │ (compose net) │   │ Keycloak/    │
└────────┘   └───────────────┘   └──────────────┘   │ Auth0        │
                    │                ▲               └──────┬───────┘
                    └───── OIDC login / token exchange ─────┘
```

- The proxy is the only host-exposed service. Brain Server binds inside the
  compose network (`brain-server:8765`), never published to the host.
- `BIND_HOST=0.0.0.0` + `BIND_PUBLIC=1` are set **inside the container only**
  (required to be reachable from the proxy); the host port mapping stays
  `127.0.0.1` — see `docker-compose.yml`.

## Option A — OAuth2-Proxy (compose profile `sso`)

Already wired in `docker-compose.yml`:

```bash
export OIDC_ISSUER_URL=https://login.microsoftonline.com/<tenant>/v2.0
export OIDC_CLIENT_ID=<client-id>
export OIDC_CLIENT_SECRET=<client-secret>
export OAUTH2_PROXY_COOKIE_SECRET=$(python3 -c "import secrets;print(secrets.token_hex(32))")
docker compose --profile sso up -d
```

- Proxy listens on `127.0.0.1:4180`; brain-server is reachable only on the
  internal network.
- `OAUTH2_PROXY_SET_AUTHORIZATION_HEADER=true` forwards the IdP session; with
  JWT mode enabled on the server, brain-server validates the forwarded token.

### JWT passthrough (JWT mode behind the proxy)

To make brain-server validate the IdP's tokens itself:

1. Set `BRAIN_JWT_ISSUER` to the IdP issuer (e.g. the Entra v2.0 issuer).
2. Export the IdP's public signing key(s) as PEM into `./data/keys` (the
   `BRAIN_UMP_KEY_DIR` volume). Key rotation at the IdP means adding the new
   PEM; the server picks up key-dir changes on reload.

This gives per-request AuthZ + audit without the proxy doing token surgery.
Opaque bearer mode remains the simpler default: the proxy authenticates, and
the server's own `AUTH_TOKEN` (from `./data/auth-token`) is what the proxy
cannot see past — set both and you get defense in depth.

## Option B — Caddy forward-auth

Caddy terminates TLS and delegates auth to any OIDC provider:

```caddy
brain.example.com {
    forward_auth localhost:9080 {
        uri /oauth2/auth
        copy_headers Authorization
    }
    reverse_proxy brain-server:8765
}
```

Run `caddy` with the `caddy-security` plugin (or an OAuth2-Proxy sidecar
listening on `:9080`) — the `copy_headers` directive forwards the IdP token to
brain-server, which validates it in JWT mode.

## Option C — Authentik (full identity platform)

Authentik as IdP + outpost proxy: users get a self-hosted login portal,
MFA/WebAuthn, and SAML bridging. The Authentik proxy outpost forwards
authenticated requests to `http://brain-server:8765` with the
`X-Authentik-*` headers; map the principal to a bearer token or enable JWT
mode and validate the forwarded token as in Option A.

## IdP matrix

| IdP | OIDC | Notes |
|---|---|---|
| Entra ID (Azure AD) | ✅ v2.0 | `--oidc-issuer-url=https://login.microsoftonline.com/<tenant>/v2.0` |
| Okta | ✅ | org URL issuer; app must allow the proxy callback |
| Keycloak | ✅ | realm URL issuer; also bridges SAML providers |
| Auth0 | ✅ | tenant issuer; add the proxy callback to the app |

## Principal handoff

- The proxy establishes **who** (IdP subject / email).
- Brain Server enforces **what** (AuthZ matrix in JWT mode; bearer token in
  opaque mode).
- Tenant isolation: `tenant_id` + `access_scope` on recall/audit rows already
  exist server-side (v1.14 M4); per-tenant quotas/rate limits are v2.0 B4.

## Security notes

- Keep the server's own auth ON behind the proxy (bearer token or JWT mode).
  The proxy authenticates the human; the server authenticates the caller.
- TLS terminates at the proxy — brain-server speaks plain HTTP on the internal
  network only.
- `no-new-privileges`, `read_only: true`, `cap_drop: ALL` are set in compose
  for both services.
- Do NOT publish brain-server's port to the host when the SSO profile is up;
  the proxy is the only ingress.

## What this does NOT do (honest limits)

- No native OIDC login screen in the client (v1.20 B2 — client login redirect,
  PKCE, external JWKS fetch).
- No SCIM provisioning (v2.0 B3).
- No SAML endpoint in the server — SAML orgs bridge via Authentik/Keycloak.
