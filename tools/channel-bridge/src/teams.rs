//! Microsoft Teams adapter — the Bot Framework + Adaptive Cards edge.
//!
//! LAW (`teams_uses_bot_framework_not_deprecated_connectors`): activities
//! arrive over the SUPPORTED Bot Framework messaging endpoint only. The
//! deprecated O365-connector path (the Office-365-style connectors and any
//! incoming-webhook-style shape) is deliberately NOT implemented — pinned
//! by a source-text doc-grep so procurement reviewers can verify.
//!
//! Auth law: EVERY `POST /messaging` is verified BEFORE a single byte of
//! body is parsed — RS256 against the `login.botframework.com` JWKS
//! (cached ≤ 1h, refetched once on an unknown `kid`), `iss` pinned to
//! `https://api.botframework.com`, `aud` pinned to the configured
//! `bot_app_id`. Any verification failure is a 401 with the body unread.
//!
//! Digest law: proposal cards are Adaptive Cards whose `Action.Submit`
//! payloads carry the digest; `console::handle_button` refuses anything
//! the render cache has not seen, before any kernel relay.
//!
//! Outbound law: client-credentials tokens for the Bot Connector API are
//! cached until ~5 minutes before expiry; drained envelopes deliver to the
//! conservative regional service host (`smba.trafficmanager.net`, a
//! documented ceiling — serviceUrl is normally echoed per-activity).
//! Identity law: actor_ref is ALWAYS the raw platform id (`from.id`),
//! never a display name; bot-authored activities never become notes.

use crate::App;
use crate::console;
use crate::render::{self, NoteDraft};
use anyhow::{Context, Result};
use axum::response::IntoResponse;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const JWKS_URL: &str = "https://login.botframework.com/v1/.well-known/keys";
const JWKS_TTL_SECS: i64 = 3_600;
const BF_ISSUER: &str = "https://api.botframework.com";
const TOKEN_REFRESH_MARGIN_SECS: i64 = 300;
const GRAPH_SCOPE: &str = "https://graph.microsoft.com/.default";
const BF_SCOPE: &str = "https://api.botframework.com/.default";
/// Pagination ceilings (bounds law): ≤ 5 pages per Graph listing.
const MAX_GRAPH_PAGES: usize = 5;
const MAX_TEAMS_LISTED: usize = 32;
const MAX_ITEMS_PER_PAGE: usize = 100;

/// `application/x-www-form-urlencoded` value encoding (unreserved bytes
/// pass through; everything else percent-encoded).
fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ── Inbound: the Bot Framework JWT verifier ────────────────────────────────

struct CachedJwks {
    keys: Arc<JwkSet>,
    fetched_at: i64,
}

/// Verifies Bot Framework activity JWTs. Nothing about this type renders:
/// verified tokens are evidence, and the JWKS is trust anchor state.
pub(crate) struct BfVerifier {
    http: reqwest::Client,
    app_id: String,
    cache: Mutex<Option<CachedJwks>>,
}

impl BfVerifier {
    pub(crate) fn new(http: reqwest::Client, app_id: String) -> Self {
        Self {
            http,
            app_id,
            cache: Mutex::new(None),
        }
    }

    async fn fetch_keys(&self) -> Result<JwkSet> {
        let v: Value = self
            .http
            .get(JWKS_URL)
            .send()
            .await
            .context("bot framework jwks endpoint unreachable")?
            .error_for_status()
            .context("bot framework jwks endpoint refused")?
            .json()
            .await
            .context("bot framework jwks returned non-json")?;
        serde_json::from_value(v).context("bot framework jwks unparsable")
    }

    async fn keys(&self, force: bool) -> Result<Arc<JwkSet>> {
        let now = chrono::Utc::now().timestamp();
        {
            // Scoped: the std MutexGuard must drop BEFORE any await (the
            // handler futures have to stay Send).
            let guard = self
                .cache
                .lock()
                .map_err(|_| anyhow::anyhow!("jwks cache mutex poisoned"))?;
            if !force
                && let Some(c) = guard.as_ref()
                && now.saturating_sub(c.fetched_at) < JWKS_TTL_SECS
            {
                return Ok(c.keys.clone());
            }
        }
        let fresh = Arc::new(self.fetch_keys().await?);
        let mut guard = self
            .cache
            .lock()
            .map_err(|_| anyhow::anyhow!("jwks cache mutex poisoned"))?;
        *guard = Some(CachedJwks {
            keys: fresh.clone(),
            fetched_at: now,
        });
        Ok(fresh)
    }

    /// Verify ONE activity token: RS256, kid-resolved against the JWKS
    /// (one forced refresh on unknown kid for rotation), issuer and
    /// audience pinned. Expiry is validated by default.
    pub(crate) async fn verify(&self, token: &str) -> Result<()> {
        let header = decode_header(token).context("bot framework token header unparsable")?;
        if header.alg != Algorithm::RS256 {
            anyhow::bail!("only RS256 bot framework tokens are accepted");
        }
        let Some(kid) = header.kid else {
            anyhow::bail!("bot framework token carries no kid");
        };
        let jwks = self.keys(false).await?;
        let refreshed: Arc<JwkSet>;
        let jwk = match jwks.find(&kid) {
            Some(j) => j,
            None => {
                // Key rotation: one forced refresh, then refuse.
                refreshed = self.keys(true).await?;
                refreshed
                    .find(&kid)
                    .context("unknown bot framework token kid after refresh")?
            }
        };
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[BF_ISSUER]);
        validation.set_audience(&[self.app_id.as_str()]);
        let key = DecodingKey::from_jwk(jwk).context("bot framework jwk unusable")?;
        decode::<Value>(token, &key, &validation).context("bot framework token rejected")?;
        Ok(())
    }
}

// ── Outbound: the Bot Connector client ──────────────────────────────────

struct CachedToken {
    scope: String,
    value: String,
    expires_at: i64,
}

/// Client-credentials token holder + activity sender. NOT `Debug`: the
/// client secret bytes and bearer values must never render.
pub(crate) struct BfClient {
    http: reqwest::Client,
    tenant: String,
    app_id: String,
    password: Vec<u8>,
    connector_slot: Mutex<Option<CachedToken>>,
    graph_slot: Mutex<Option<CachedToken>>,
}

impl BfClient {
    pub(crate) fn new(
        http: reqwest::Client,
        tenant: String,
        app_id: String,
        password: Vec<u8>,
    ) -> Self {
        Self {
            http,
            tenant,
            app_id,
            password,
            connector_slot: Mutex::new(None),
            graph_slot: Mutex::new(None),
        }
    }

    async fn fetch_token(&self, scope: &str) -> Result<(String, i64)> {
        // reqwest is built default-features-off here (no urlencoded form
        // helper), so the client-credentials body is encoded by hand.
        let password = String::from_utf8_lossy(&self.password).to_string();
        let body = format!(
            "grant_type=client_credentials&client_id={}&client_secret={}&scope={}",
            form_encode(&self.app_id),
            form_encode(&password),
            form_encode(scope),
        );
        let resp: Value = self
            .http
            .post(format!(
                "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
                self.tenant
            ))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .context("identity token endpoint unreachable")?
            .error_for_status()
            .context("identity token endpoint refused")?
            .json()
            .await
            .context("identity token response not json")?;
        let value = resp
            .get("access_token")
            .and_then(|x| x.as_str())
            .context("token response missing access_token")?
            .to_string();
        let ttl = resp
            .get("expires_in")
            .and_then(|x| x.as_i64())
            .unwrap_or(3_600)
            .clamp(60, 86_400);
        Ok((value, ttl))
    }

    async fn cached_token(&self, slot: &Mutex<Option<CachedToken>>, scope: &str) -> Result<String> {
        let now = chrono::Utc::now().timestamp();
        // Scoped: the std MutexGuard must drop BEFORE any await (the
        // handler futures have to stay Send).
        {
            let guard = slot
                .lock()
                .map_err(|_| anyhow::anyhow!("token cache mutex poisoned"))?;
            if let Some(t) = guard.as_ref()
                && t.scope == scope
                && t.expires_at.saturating_sub(now) > TOKEN_REFRESH_MARGIN_SECS
            {
                return Ok(t.value.clone());
            }
        }
        let (value, ttl) = self.fetch_token(scope).await?;
        let mut guard = slot
            .lock()
            .map_err(|_| anyhow::anyhow!("token cache mutex poisoned"))?;
        *guard = Some(CachedToken {
            scope: scope.to_string(),
            expires_at: now.saturating_add(ttl),
            value: value.clone(),
        });
        Ok(value)
    }

    async fn connector_token(&self) -> Result<String> {
        self.cached_token(&self.connector_slot, BF_SCOPE).await
    }

    async fn graph_token(&self) -> Result<String> {
        self.cached_token(&self.graph_slot, GRAPH_SCOPE).await
    }

    /// Deliver one proactive activity: `POST {service}/v3/conversations/
    /// {conversation_ref}/activities`. serviceUrl MUST be https (the
    /// inbound activity is Bot-Framework-verified; the drain path uses the
    /// conservative regional host — see the README ceiling note).
    pub(crate) async fn send_activity(
        &self,
        service_url: &str,
        conversation_ref: &str,
        text: &str,
    ) -> Result<()> {
        let base = service_url.trim().trim_end_matches('/');
        if !base.starts_with("https://") {
            anyhow::bail!("bot connector service url must be https");
        }
        if !render::bounded_ref(conversation_ref) {
            anyhow::bail!("conversation ref unbounded; refusing delivery");
        }
        let token = self.connector_token().await?;
        let url = format!("{base}/v3/conversations/{conversation_ref}/activities");
        let resp = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&json!({"type": "message", "text": text}))
            .send()
            .await
            .context("bot connector endpoint unreachable")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "bot connector refused activity ({}): {}",
                status,
                console::snippet_text(&body, 200)
            );
        }
        Ok(())
    }

    /// Deliver one Adaptive Card attachment (the proposal render path):
    /// `POST {service}/v3/conversations/{conversation_ref}/activities` with
    /// the card as the sole attachment.
    pub(crate) async fn send_card(
        &self,
        service_url: &str,
        conversation_ref: &str,
        card: &Value,
    ) -> Result<()> {
        let base = service_url.trim().trim_end_matches('/');
        if !base.starts_with("https://") {
            anyhow::bail!("bot connector service url must be https");
        }
        if !render::bounded_ref(conversation_ref) {
            anyhow::bail!("conversation ref unbounded; refusing delivery");
        }
        let token = self.connector_token().await?;
        let url = format!("{base}/v3/conversations/{conversation_ref}/activities");
        let resp = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&json!({"type": "message", "attachments": [card]}))
            .send()
            .await
            .context("bot connector endpoint unreachable")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "bot connector refused card ({}): {}",
                status,
                console::snippet_text(&body, 200)
            );
        }
        Ok(())
    }
}

// ── Shared runtime state ────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct TeamsState {
    pub(crate) app: App,
    pub(crate) client: Arc<BfClient>,
    pub(crate) verifier: Arc<BfVerifier>,
    pub(crate) cache: Arc<console::RenderCache>,
}

// ── Inbound handler ─────────────────────────────────────────────────────────

fn deny_401(detail: &'static str) -> axum::response::Response {
    tracing::warn!(detail, "teams inbound refused BEFORE parse");
    (axum::http::StatusCode::UNAUTHORIZED, "unauthorized").into_response()
}

fn ok_empty() -> axum::response::Response {
    // Bot Framework retries aggressively; always answer 200 {} quickly.
    (axum::http::StatusCode::OK, axum::Json(json!({}))).into_response()
}

/// `POST /messaging` — verify FIRST (Bot Framework JWT), parse SECOND.
/// `type == "message"` with text becomes a case note; `type == "message"`
/// with only a `value` object is an Adaptive Card `Action.Submit` and
/// obeys the digest law. Everything else is ignored with a 200.
pub(crate) async fn messaging(
    axum::extract::State(st): axum::extract::State<TeamsState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Some(raw) = headers.get("authorization").and_then(|h| h.to_str().ok()) else {
        return deny_401("missing authorization header");
    };
    let Some(token) = raw.strip_prefix("Bearer ").filter(|t| !t.is_empty()) else {
        return deny_401("authorization header is not a bot framework bearer token");
    };
    if let Err(e) = st.verifier.verify(token).await {
        tracing::error!("bot framework token verification FAILED (body never parsed): {e:#}");
        return deny_401("token verification failed");
    }

    let activity: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("verified body was not a bot framework activity: {e}");
            return ok_empty();
        }
    };

    let activity_text = activity
        .get("text")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if activity_text == "brain pending" {
        // THE RENDER PATH: the operator asks for pending proposals in-channel
        // (mention-stripped text). Cards render with their digests and the
        // cache remembers them; this message is NOT a case note.
        handle_pending(&st, &activity).await;
        return ok_empty();
    }
    let has_text = activity
        .get("text")
        .and_then(|x| x.as_str())
        .map(|t| !t.trim().is_empty())
        .unwrap_or(false);
    if has_text {
        handle_note(&st, &activity).await;
    } else if let Some(value) = activity.get("value").filter(|v| v.is_object()) {
        handle_card_submit(&st, &activity, value).await;
    }
    ok_empty()
}

/// THE RENDER PATH: relay `pending`, render each proposal as an Adaptive
/// Card carrying its digest, post it into the asking conversation, and
/// remember the digest so later Action.Submit clicks can bind. Without
/// this path the render cache would never fill and the digest law would
/// refuse every approval (fail-closed, by design).
async fn handle_pending(st: &TeamsState, activity: &Value) {
    let Ok(cfg) = st.app.cfg.teams() else {
        return;
    };
    let Some(conversation) = activity
        .pointer("/conversation/id")
        .and_then(|x| x.as_str())
        .filter(|c| render::bounded_ref(c))
        .map(str::to_string)
    else {
        return;
    };
    if !cfg.mapped_channels.iter().any(|m| m == &conversation) {
        tracing::debug!(
            conversation,
            "pending request from an unmapped conversation; dropped"
        );
        return;
    }
    let service_url = activity
        .get("serviceUrl")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let proposals = match console::fetch_pending(
        &st.app.http,
        &st.app.brain_url,
        &st.app.cfg.kind,
        &st.app.cfg.webhook_secret,
        console::MAX_PENDING_LIMIT,
    )
    .await
    {
        Ok(p) if p.is_empty() => {
            if let Err(e) = st
                .client
                .send_activity(&service_url, &conversation, "nothing pending")
                .await
            {
                tracing::error!("teams pending reply failed loudly: {e:#}");
            }
            return;
        }
        Ok(p) => p,
        Err(e) => {
            tracing::error!("teams pending relay failed loudly: {e:#}");
            let _ = st
                .client
                .send_activity(
                    &service_url,
                    &conversation,
                    &format!(
                        "kernel refused: {}",
                        console::snippet_text(&format!("{e:#}"), 160)
                    ),
                )
                .await;
            return;
        }
    };
    for p in &proposals {
        st.cache.remember(p.id, &p.digest);
        let card = render::adaptive_card(p);
        if let Err(e) = st
            .client
            .send_card(&service_url, &conversation, &card)
            .await
        {
            tracing::error!(proposal_id = p.id, "teams card render failed loudly: {e:#}");
        }
    }
}

async fn handle_note(st: &TeamsState, activity: &Value) {
    let Ok(cfg) = st.app.cfg.teams() else {
        return;
    };
    let Some(draft) = project_activity(&cfg.mapped_channels, &cfg.bot_app_id, activity) else {
        return;
    };
    let envelope = crate::envelope_json(&draft);
    match crate::forward_envelope(&st.app, &envelope).await {
        Ok(code) => tracing::debug!(code, "teams envelope forwarded to kernel"),
        Err(e) => tracing::error!("teams forward failed (loud): {e:#}"),
    }
}

/// PURE projection: a mapped human `message` activity → the locked
/// envelope draft. `from.name` and every other display-name field is NEVER
/// read; actor_ref is the raw `from.id` and nothing else.
pub(crate) fn project_activity(
    mapped_channels: &[String],
    bot_app_id: &str,
    activity: &Value,
) -> Option<NoteDraft> {
    if activity.get("type").and_then(|x| x.as_str()) != Some("message") {
        return None;
    }
    let text = activity
        .get("text")
        .and_then(|x| x.as_str())
        .filter(|t| !t.trim().is_empty())?;
    let conversation = activity
        .pointer("/conversation/id")
        .and_then(|x| x.as_str())
        .filter(|c| render::bounded_ref(c))?;
    if !mapped_channels.iter().any(|m| m == conversation) {
        tracing::debug!(
            conversation,
            "unmapped teams conversation dropped (never crosses)"
        );
        return None;
    }
    let role = activity
        .pointer("/from/role")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let from_id = activity
        .pointer("/from/id")
        .and_then(|x| x.as_str())
        .filter(|f| render::bounded_ref(f))?;
    // Bot-authored activities never become case notes.
    if role == "bot" || from_id == bot_app_id {
        return None;
    }
    let external_id = activity
        .get("id")
        .and_then(|x| x.as_str())
        .filter(|i| render::bounded_ref(i))?;
    Some(NoteDraft {
        conversation_ref: conversation.to_string(),
        text: render::sanitize_text(text),
        external_id: external_id.to_string(),
        actor_ref: from_id.to_string(),
    })
}

async fn handle_card_submit(st: &TeamsState, activity: &Value, value: &Value) {
    let Some((approve, proposal_id, digest)) = render::parse_card_submit(value) else {
        tracing::debug!("unparseable adaptive card submit ignored");
        return;
    };
    let Some(actor_ref) = activity
        .pointer("/from/id")
        .and_then(|x| x.as_str())
        .filter(|f| render::bounded_ref(f))
        .map(str::to_string)
    else {
        tracing::warn!("adaptive card submit without a bounded actor; refused");
        return;
    };
    let conversation = activity
        .pointer("/conversation/id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let service_url = activity
        .get("serviceUrl")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let app = st.app.clone();
    let actor = actor_ref.clone();
    let relay = move |approve: bool, proposal_id: i64, digest: String| {
        let app = app.clone();
        let actor = actor.clone();
        async move {
            console::post_console(
                &app.http,
                &app.brain_url,
                &app.cfg.kind,
                &app.cfg.webhook_secret,
                &console::body_decide(approve, proposal_id, &digest, &actor),
            )
            .await
        }
    };
    let verdict =
        console::handle_button(&st.cache, approve, proposal_id, &digest, &actor_ref, relay).await;

    // Best-effort confirmation back into the conversation (the 200 {} has
    // no body for BF — the card reply IS the operator-facing receipt).
    if verdict.relayed
        && !service_url.is_empty()
        && render::bounded_ref(&conversation)
        && let Err(e) = st
            .client
            .send_activity(&service_url, &conversation, &verdict.reply_text)
            .await
    {
        tracing::error!("teams confirmation delivery failed loudly: {e:#}");
    }
}

// ── Drain crank ─────────────────────────────────────────────────────────

/// One teams drain crank: deliver drained envelopes to the conservative
/// regional service host and route handover pings (case channel, else the
/// configured handover channel, else drop-with-warn). Failures log loud.
pub(crate) async fn crank(st: &TeamsState) -> Result<()> {
    let batch = crate::outbound::drain_batch(&st.app).await?;
    let cfg = st.app.cfg.teams()?;
    let default_service = format!("https://smba.trafficmanager.net/{}", cfg.bot_tenant_id);
    for env in batch.envelopes.iter().take(64) {
        let Some(conversation_ref) = env
            .get("conversation_ref")
            .and_then(|x| x.as_str())
            .filter(|c| render::bounded_ref(c))
        else {
            tracing::warn!("drained envelope without a bounded conversation_ref; dropped");
            continue;
        };
        if !cfg.mapped_channels.iter().any(|m| m == conversation_ref) {
            tracing::warn!(
                conversation_ref,
                "drained envelope for an unmapped conversation; dropped"
            );
            continue;
        }
        let text = env
            .get("text")
            .and_then(|x| x.as_str())
            .map(render::sanitize_text)
            .unwrap_or_default();
        if let Err(e) = st
            .client
            .send_activity(&default_service, conversation_ref, &text)
            .await
        {
            tracing::error!(conversation_ref, "teams delivery failed loudly: {e:#}");
        }
    }
    for ping in batch.pings.iter().take(32) {
        let Some((case_target, text)) = render::build_ping_message_teams(ping) else {
            continue;
        };
        let Some(target) = render::ping_target(
            case_target.as_deref().unwrap_or(""),
            cfg.handover_channel.as_deref(),
        ) else {
            tracing::warn!("handover ping with no case channel and no handover_channel; dropped");
            continue;
        };
        if let Err(e) = st
            .client
            .send_activity(&default_service, &target, &text)
            .await
        {
            tracing::error!(target, "handover ping delivery failed loudly: {e:#}");
        }
    }
    Ok(())
}

// ── Admin-gated Graph enumeration ───────────────────────────────────────

/// `--list-channels`: read-only enumeration of the operator's joined
/// teams and their channels via Graph (client credentials, Graph scope),
/// printing `id<TAB>name` lines for the operator to copy into
/// `mapped_channels`. NEVER writes anything.
pub(crate) async fn list_channels(app: &App) -> Result<()> {
    let cfg = app.cfg.teams()?;
    let client = BfClient::new(
        app.http.clone(),
        cfg.bot_tenant_id.clone(),
        cfg.bot_app_id.clone(),
        cfg.bot_password.clone(),
    );
    let token = client.graph_token().await?;

    let mut url = "https://graph.microsoft.com/v1.0/me/joinedTeams".to_string();
    let mut team_ids: Vec<String> = Vec::new();
    for _page in 0..MAX_GRAPH_PAGES {
        let resp: Value = app
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .context("graph joinedTeams unreachable")?
            .error_for_status()
            .context("graph joinedTeams refused")?
            .json()
            .await
            .context("graph joinedTeams not json")?;
        for team in resp
            .get("value")
            .and_then(|x| x.as_array())
            .into_iter()
            .flatten()
            .take(MAX_ITEMS_PER_PAGE)
        {
            if team_ids.len() >= MAX_TEAMS_LISTED {
                break;
            }
            if let Some(id) = team.get("id").and_then(|x| x.as_str()) {
                team_ids.push(id.to_string());
            }
        }
        match resp.get("@odata.nextLink").and_then(|x| x.as_str()) {
            Some(next) if team_ids.len() < MAX_TEAMS_LISTED => url = next.to_string(),
            _ => break,
        }
    }

    for team_id in &team_ids {
        let mut chan_url = format!("https://graph.microsoft.com/v1.0/teams/{team_id}/channels");
        for _page in 0..MAX_GRAPH_PAGES {
            let resp: Value = app
                .http
                .get(&chan_url)
                .bearer_auth(&token)
                .send()
                .await
                .context("graph channels unreachable")?
                .error_for_status()
                .context("graph channels refused")?
                .json()
                .await
                .context("graph channels not json")?;
            for ch in resp
                .get("value")
                .and_then(|x| x.as_array())
                .into_iter()
                .flatten()
                .take(MAX_ITEMS_PER_PAGE)
            {
                let id = ch.get("id").and_then(|x| x.as_str()).unwrap_or("");
                let name = ch.get("displayName").and_then(|x| x.as_str()).unwrap_or("");
                if !id.is_empty() {
                    println!("{id}\t{}", console::snippet_text(name, 120));
                }
            }
            match resp.get("@odata.nextLink").and_then(|x| x.as_str()) {
                Some(next) => chan_url = next.to_string(),
                _ => break,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::panic,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]
    use super::*;

    fn hex64() -> String {
        // Exactly 64 hex chars, built (not hand-typed).
        "0f1e2d3c".repeat(8)
    }

    fn proposal() -> render::Proposal {
        render::Proposal {
            id: 7,
            kind: "channel/template".to_string(),
            content: "appointment reminder".to_string(),
            digest: hex64(),
        }
    }

    // ── HERALD PIN: adaptive_card_submit_returns_digest — the card's
    //    Action.Submit payloads round-trip id+digest exactly, and the
    //    digest law accepts the cached digest but refuses a foreign one.
    #[test]
    fn adaptive_card_submit_returns_digest() {
        let hex = hex64();
        let card = render::adaptive_card(&proposal());
        assert_eq!(
            card["contentType"],
            "application/vnd.microsoft.card.adaptive"
        );
        assert_eq!(card["content"]["version"], "1.4");

        let (approve, id, digest) =
            render::parse_card_submit(&card["content"]["actions"][0]["data"]).unwrap();
        assert!(approve && id == 7 && digest == hex);
        let (reject, id2, digest2) =
            render::parse_card_submit(&card["content"]["actions"][1]["data"]).unwrap();
        assert!(!reject && id2 == 7 && digest2 == hex);

        let cache = console::RenderCache::new();
        cache.remember(id, &digest);
        assert!(matches!(
            console::digest_gate(&cache, id, &digest),
            console::DigestVerdict::Bound
        ));
        // Foreign digest (well-formed, never rendered) → REFUSED.
        let foreign = format!("{:064x}", 0xdead_beefu64);
        assert!(matches!(
            console::digest_gate(&cache, id, &foreign),
            console::DigestVerdict::Refused(_)
        ));
    }

    // ── HERALD PIN (doc-grep): teams_uses_bot_framework_not_deprecated_
    //    connectors — the source and the README speak the SUPPORTED route
    //    and never name the deprecated paths.
    #[test]
    fn teams_uses_bot_framework_not_deprecated_connectors() {
        // NOTE: the needles are assembled from fragments — this file must
        // not contain them literally, or the doc-grep would find itself.
        let deprecated = [
            format!("o365 {}", "connector"),
            format!("office 365 {}", "connector"),
            format!("incoming {}", "webhook"),
        ];
        let src = include_str!("../src/teams.rs");
        let readme = include_str!("../README.md");
        for doc in [src, readme] {
            assert!(
                doc.contains("Bot Framework"),
                "doc must speak Bot Framework"
            );
            assert!(
                doc.contains("Adaptive Cards"),
                "doc must speak Adaptive Cards"
            );
            let lower = doc.to_lowercase();
            for needle in &deprecated {
                assert!(
                    !lower.contains(needle.as_str()),
                    "deprecated shape absent: {needle}"
                );
            }
        }
    }

    #[test]
    fn teams_projection_skips_bots_and_unmapped() {
        let mapped = vec!["19:abc@thread.tacv2".to_string()];
        let activity = json!({
            "type": "message",
            "id": "act-1",
            "from": {"id": "29:human1", "name": "ignored"},
            "conversation": {"id": "19:abc@thread.tacv2"},
            "text": "ping"
        });
        let draft = project_activity(&mapped, "bot-app", &activity)
            .expect("mapped human activity projects");
        assert_eq!(draft.conversation_ref, "19:abc@thread.tacv2");
        assert_eq!(draft.actor_ref, "29:human1");

        let bot = json!({
            "type": "message", "id": "act-2",
            "from": {"id": "bot-app", "role": "bot"},
            "conversation": {"id": "19:abc@thread.tacv2"},
            "text": "beep"
        });
        assert!(project_activity(&mapped, "bot-app", &bot).is_none());

        let unmapped = json!({
            "type": "message", "id": "act-3",
            "from": {"id": "29:human1"},
            "conversation": {"id": "19:other@thread.tacv2"},
            "text": "sneak"
        });
        assert!(project_activity(&mapped, "bot-app", &unmapped).is_none());
    }
}
