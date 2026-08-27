//! The bridge config: `channel-{kind}-{tenant}.json`, owner-only (0600),
//! the SAME shared substrate file the kernel reads (domain +
//! webhook_secret) enriched with the per-kind edge keys
//! (`whatsapp` | `slack` | `teams`). Fail-closed everywhere: bad perms,
//! unreadable secrets, wrong shapes, unknown kinds, or absurd sizes refuse
//! at LOAD — never silently run. Per-kind required fields are enforced
//! HERE: a whatsapp config without its Meta keys is dead on arrival, a
//! slack config without `mapped_channels` must never boot, a teams config
//! without its Bot Framework app ids is refused before any socket opens.
//!
//! Secret hygiene: tokens live in 0600 FILES, never inline; the sub-config
//! types deliberately carry no `Debug` derive so secret bytes can never
//! render in logs, panics, or traces.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;

const MAX_SECRET_BYTES: usize = 256;
const MAX_CHANNELS: usize = 16;
const MAX_SLACK_CHANNEL_ID_LEN: usize = 32;
const MAX_TEAMS_CONVERSATION_ID_LEN: usize = 128;
const MAX_GUIDISH_LEN: usize = 64;

#[derive(Clone)]
pub(crate) struct WhatsAppCfg {
    /// Meta subscription verify token (handshake echo gate).
    pub(crate) verify_token: String,
    /// Cloud API phone-number id.
    pub(crate) phone_number_id: String,
    /// Meta app secret bytes (hub signature verification).
    pub(crate) app_secret: Vec<u8>,
    /// Permanent system-user token bytes (Cloud API sends).
    pub(crate) access_token: Vec<u8>,
    /// Graph API version stamp (`v21.0` etc., pinned per deployment).
    pub(crate) graph_api_version: String,
}

/// Slack edge config: the xapp- app token is used ONLY for
/// `apps.connections.open`; the xoxb- bot token ONLY for `chat.postMessage`
/// (least privilege at the workspace-app level — see the README).
#[derive(Clone)]
pub(crate) struct SlackCfg {
    pub(crate) mapped_channels: Vec<String>,
    /// Roomless-case handover pings fall back here; absent = dropped+warned.
    pub(crate) handover_channel: Option<String>,
    pub(crate) app_token: Vec<u8>,
    pub(crate) bot_token: Vec<u8>,
}

/// Teams edge config: Bot Framework registration identity (client
/// credentials) + the mapped conversation ids.
#[derive(Clone)]
pub(crate) struct TeamsCfg {
    pub(crate) bot_app_id: String,
    pub(crate) bot_tenant_id: String,
    pub(crate) bot_password: Vec<u8>,
    pub(crate) mapped_channels: Vec<String>,
    /// Roomless-case handover pings fall back here; absent = dropped+warned.
    pub(crate) handover_channel: Option<String>,
}

#[derive(Clone)]
pub(crate) struct BridgeConfig {
    /// The `{kind}` route segment (whatsapp | slack | teams).
    pub(crate) kind: String,
    /// The `{tenant}` segment of the config filename.
    pub(crate) tenant: String,
    /// The registered domain every case under this bridge lives in.
    pub(crate) domain: String,
    /// Standard-Webhooks secret (kernel seam signing; the ONLY kernel
    /// credential this process holds).
    pub(crate) webhook_secret: Vec<u8>,
    whatsapp: Option<WhatsAppCfg>,
    slack: Option<SlackCfg>,
    teams: Option<TeamsCfg>,
    /// SHA-256 of the FULL config-file bytes (mount evidence; the kernel
    /// recomputes it from its own copy — neither side self-certifies).
    pub(crate) config_sha256: String,
}

fn read_perm_file(p: &Path) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(p).with_context(|| format!("stat {}", p.display()))?;
        if meta.permissions().mode() & 0o077 != 0 {
            anyhow::bail!("{} must be owner-only (0600); refusing", p.display());
        }
    }
    let mut bytes = std::fs::read(p).with_context(|| format!("read {}", p.display()))?;
    if bytes.len() > MAX_SECRET_BYTES {
        anyhow::bail!("{} exceeds {MAX_SECRET_BYTES} bytes; refusing", p.display());
    }
    while matches!(bytes.last(), Some(b'\n') | Some(b'\r') | Some(b' ')) {
        bytes.pop();
    }
    if bytes.is_empty() {
        anyhow::bail!("{} is empty after trim; refusing", p.display());
    }
    Ok(bytes)
}

fn str_field(v: &Value, key: &str) -> Result<String> {
    let s = v
        .get(key)
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .context(format!("missing/oversized string field {key}"))?;
    Ok(s.to_string())
}

fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_domain(d: &str) -> bool {
    !d.is_empty()
        && d.len() <= 63
        && d.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Slack channel ids: `[A-Z0-9]+`, ≤ 32 chars. Lowercase junk, control
/// chars, and overlong ids refuse at load.
fn valid_slack_channel_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_SLACK_CHANNEL_ID_LEN
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Teams conversation ids are opaque service strings (`19:…@thread.tacv2`):
/// bounded, control-char-free. Shape policing beyond that is the kernel's.
fn valid_teams_conversation_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_TEAMS_CONVERSATION_ID_LEN && !s.chars().any(char::is_control)
}

/// GUID-ish Azure identity strings: alphanumeric + hyphens, ≤ 64.
fn valid_guidish(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_GUIDISH_LEN
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Load a REQUIRED string-array field (1..=max_items items, each passing
/// `shape`).
fn string_array(
    v: &Value,
    key: &str,
    max_items: usize,
    shape: fn(&str) -> bool,
) -> Result<Vec<String>> {
    let arr = v
        .get(key)
        .and_then(|x| x.as_array())
        .context(format!("missing array field {key}"))?;
    if arr.is_empty() || arr.len() > max_items {
        bail!("{key} must carry 1..={max_items} items");
    }
    let mut out = Vec::new();
    for item in arr {
        let s = item
            .as_str()
            .context(format!("{key} items must be strings"))?;
        if !shape(s) {
            bail!("{key} item fails the shape check: {s:?}");
        }
        out.push(s.to_string());
    }
    Ok(out)
}

/// Load an OPTIONAL single-channel field, shape-checked when present.
fn opt_channel(v: &Value, key: &str, shape: fn(&str) -> bool) -> Result<Option<String>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(x) => {
            let s = x.as_str().context(format!("{key} must be a string"))?;
            if !shape(s) {
                bail!("{key} fails the shape check: {s:?}");
            }
            Ok(Some(s.to_string()))
        }
    }
}

/// Resolve a PATH-typed config value relative to the CONFIG DIR when relative.
fn resolve_path(config_path: &Path, p: &str) -> Result<std::path::PathBuf> {
    let candidate = Path::new(p);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        config_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(candidate)
    };
    for comp in resolved.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            anyhow::bail!("secret paths may not traverse upward ({p})");
        }
    }
    Ok(resolved)
}

fn graph_version_invalid(v: &Value) -> bool {
    match v.get("graph_api_version").and_then(|x| x.as_str()) {
        None => false,
        Some(s) => {
            let body = match s.get(1..) {
                Some(b) if s.starts_with('v') && !b.is_empty() => b,
                _ => return true,
            };
            !body
                .split('.')
                .all(|part| part.bytes().all(|b| b.is_ascii_digit()))
        }
    }
}

fn load_whatsapp(v: &Value, config_path: &Path) -> Result<WhatsAppCfg> {
    let verify_token = str_field(v, "verify_token")?;
    let phone_number_id = str_field(v, "phone_number_id")?;
    let app_secret_path = str_field(v, "app_secret_path")
        .map_err(|e| e.context("app_secret_path resolves where the app secret lives"))
        .and_then(|p| resolve_path(config_path, &p))?;
    let access_token_path = str_field(v, "access_token_path")
        .map_err(|e| e.context("access_token_path resolves where the system-user token lives"))
        .and_then(|p| resolve_path(config_path, &p))?;
    let app_secret = read_perm_file(&app_secret_path)?;
    let access_token = read_perm_file(&access_token_path)?;
    // Reject path-traversal resolved ABOVE against the config dir only:
    // absolute paths are allowed (operator-owned machines), but any
    // relative component that escapes is nonsense.
    if graph_version_invalid(v) {
        anyhow::bail!("graph_api_version must look like vNN.N");
    }
    let graph_api_version = v
        .get("graph_api_version")
        .and_then(|x| x.as_str())
        .unwrap_or("v21.0")
        .to_string();
    Ok(WhatsAppCfg {
        verify_token,
        phone_number_id,
        app_secret,
        access_token,
        graph_api_version,
    })
}

fn load_slack(v: &Value, config_path: &Path) -> Result<SlackCfg> {
    let mapped_channels = string_array(v, "mapped_channels", MAX_CHANNELS, valid_slack_channel_id)?;
    let handover_channel = opt_channel(v, "handover_channel", valid_slack_channel_id)?;
    let app_token_path = str_field(v, "app_token_path")
        .map_err(|e| e.context("app_token_path resolves where the Socket Mode xapp- token lives"))
        .and_then(|p| resolve_path(config_path, &p))?;
    let bot_token_path = str_field(v, "bot_token_path")
        .map_err(|e| e.context("bot_token_path resolves where the xoxb- bot token lives"))
        .and_then(|p| resolve_path(config_path, &p))?;
    let app_token = read_perm_file(&app_token_path)?;
    if !app_token.starts_with(b"xapp-") {
        bail!("app token must be a Socket Mode xapp- token; refusing");
    }
    let bot_token = read_perm_file(&bot_token_path)?;
    if !bot_token.starts_with(b"xoxb-") {
        bail!("bot token must be an xoxb- bot token; refusing");
    }
    Ok(SlackCfg {
        mapped_channels,
        handover_channel,
        app_token,
        bot_token,
    })
}

fn load_teams(v: &Value, config_path: &Path) -> Result<TeamsCfg> {
    let bot_app_id = str_field(v, "bot_app_id")?;
    if !valid_guidish(&bot_app_id) {
        bail!("bot_app_id must be a bounded GUID-ish string");
    }
    let bot_tenant_id = str_field(v, "bot_tenant_id")?;
    if !valid_guidish(&bot_tenant_id) {
        bail!("bot_tenant_id must be a bounded GUID-ish string");
    }
    let bot_password_path = str_field(v, "bot_password_path")
        .map_err(|e| e.context("bot_password_path resolves where the client secret lives"))
        .and_then(|p| resolve_path(config_path, &p))?;
    let bot_password = read_perm_file(&bot_password_path)?;
    let mapped_channels = string_array(
        v,
        "mapped_channels",
        MAX_CHANNELS,
        valid_teams_conversation_id,
    )?;
    let handover_channel = opt_channel(v, "handover_channel", valid_teams_conversation_id)?;
    Ok(TeamsCfg {
        bot_app_id,
        bot_tenant_id,
        bot_password,
        mapped_channels,
        handover_channel,
    })
}

impl BridgeConfig {
    /// Per-kind accessors: a config of the WRONG kind refuses at runtime
    /// too (defense in depth — the loader already refuses at load).
    pub(crate) fn whatsapp(&self) -> Result<&WhatsAppCfg> {
        self.whatsapp
            .as_ref()
            .context("config is not a whatsapp edge config")
    }

    pub(crate) fn slack(&self) -> Result<&SlackCfg> {
        self.slack
            .as_ref()
            .context("config is not a slack edge config")
    }

    pub(crate) fn teams(&self) -> Result<&TeamsCfg> {
        self.teams
            .as_ref()
            .context("config is not a teams edge config")
    }

    /// Load + validate one bridge config. Any suspicion refuses loudly — a
    /// misconfigured edge must be visible, never silently dark.
    pub(crate) fn load(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta =
                std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
            if meta.permissions().mode() & 0o077 != 0 {
                anyhow::bail!("config must be owner-only (0600); refusing to trust it");
            }
        }
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let v: Value =
            serde_json::from_slice(&bytes).with_context(|| "config is not valid JSON")?;

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("unrepresentable config name")?
            .to_string();
        let stem = name
            .strip_prefix("channel-")
            .and_then(|s| s.strip_suffix(".json"))
            .context("name must be channel-{kind}-{tenant}.json")?;
        let (kind, tenant) = stem
            .split_once('-')
            .context("name must carry kind and tenant segments")?;
        if !matches!(kind, "whatsapp" | "slack" | "teams") {
            bail!(
                "unsupported channel kind {kind:?} (this binary ships whatsapp|slack|teams edges)"
            );
        }
        if !valid_segment(tenant) {
            anyhow::bail!("invalid tenant segment {tenant:?}");
        }

        let domain = str_field(&v, "domain")?;
        if !valid_domain(&domain) {
            anyhow::bail!("invalid domain label {domain:?}");
        }
        let webhook_secret = str_field(&v, "webhook_secret")?.into_bytes();

        let (whatsapp, slack, teams) = match kind {
            "whatsapp" => (Some(load_whatsapp(&v, path)?), None, None),
            "slack" => (None, Some(load_slack(&v, path)?), None),
            _ => (None, None, Some(load_teams(&v, path)?)),
        };

        Ok(Self {
            kind: kind.to_string(),
            tenant: tenant.to_string(),
            domain,
            webhook_secret,
            whatsapp,
            slack,
            teams,
            config_sha256: crate::hubsig::sha256_hex(&bytes),
        })
    }
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
    use std::path::PathBuf;

    const GOOD_BODY: &[u8] = br#"{
        "domain":"acme",
        "webhook_secret":"whsec-x",
        "verify_token":"vt",
        "phone_number_id":"1234567890",
        "app_secret_path":"app_secret.txt",
        "access_token_path":"token.txt"
    }"#;

    const SLACK_BODY: &[u8] = br#"{
        "domain":"acme",
        "webhook_secret":"whsec-x",
        "app_token_path":"app_token.txt",
        "bot_token_path":"bot_token.txt",
        "mapped_channels":["C0123ABCD","C0HANDOV1"],
        "handover_channel":"C0HANDOV1"
    }"#;

    const TEAMS_BODY: &[u8] = br#"{
        "domain":"acme",
        "webhook_secret":"whsec-x",
        "bot_app_id":"00000000-0000-0000-0000-000000000001",
        "bot_tenant_id":"11111111-1111-1111-1111-111111111111",
        "bot_password_path":"bot_password.txt",
        "mapped_channels":["19:abc@thread.tacv2"]
    }"#;

    fn write_secret(dir: &Path, name: &str, content: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        p
    }

    fn write_config(dir: &Path, name: &str, body: &[u8], mode: u32) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        p
    }

    #[test]
    fn config_fail_closed_on_perms_and_traversal() {
        // World-readable config → refuse.
        let dir = tempfile::tempdir().unwrap();
        let loose = write_config(dir.path(), "channel-whatsapp-acme.json", GOOD_BODY, 0o644);
        assert!(BridgeConfig::load(&loose).is_err());

        // Upward-traversing secret path → refuse even with clean perms.
        let tight = write_config(
            dir.path(),
            "channel-whatsapp-beta.json",
            br#"{"domain":"beta","webhook_secret":"s","verify_token":"t",
                 "phone_number_id":"1","app_secret_path":"../esc.txt",
                 "access_token_path":"token.txt"}"#,
            0o600,
        );
        assert!(BridgeConfig::load(&tight).is_err());

        // Wrong KIND in filename → refuse (wrong binary for that config).
        let wrong = write_config(dir.path(), "channel-signal-acme.json", GOOD_BODY, 0o600);
        assert!(BridgeConfig::load(&wrong).is_err());
    }

    // ── HERALD PIN: the whatsapp GOOD_BODY fixture still loads — the
    //    generalization must not disturb the Caravel edge.
    #[test]
    fn whatsapp_good_body_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        write_secret(dir.path(), "app_secret.txt", "appsecret-bytes");
        write_secret(dir.path(), "token.txt", "token-bytes");
        let cfg_path = write_config(dir.path(), "channel-whatsapp-acme.json", GOOD_BODY, 0o600);
        let cfg = BridgeConfig::load(&cfg_path).expect("good whatsapp config loads");
        assert_eq!(cfg.kind, "whatsapp");
        assert_eq!(cfg.tenant, "acme");
        let wa = cfg.whatsapp().expect("whatsapp sub-config present");
        assert_eq!(wa.phone_number_id, "1234567890");
        assert_eq!(wa.graph_api_version, "v21.0", "default version applies");
        assert_eq!(wa.app_secret, b"appsecret-bytes");
        assert!(cfg.slack().is_err(), "wrong-kind accessor refuses");
        assert!(cfg.teams().is_err());
    }

    // ── HERALD PIN: slack config loads with mapped_channels; loose perms
    //    refuse; wrong token prefixes refuse.
    #[test]
    fn slack_config_loads_with_mapped_channels() {
        let dir = tempfile::tempdir().unwrap();
        write_secret(dir.path(), "app_token.txt", "xapp-1-A111-222");
        write_secret(dir.path(), "bot_token.txt", "xoxb-333-BBB");
        let cfg_path = write_config(dir.path(), "channel-slack-acme.json", SLACK_BODY, 0o600);
        let cfg = BridgeConfig::load(&cfg_path).expect("good slack config loads");
        assert_eq!(cfg.kind, "slack");
        let slack = cfg.slack().expect("slack sub-config present");
        assert_eq!(slack.mapped_channels, vec!["C0123ABCD", "C0HANDOV1"]);
        assert_eq!(slack.handover_channel.as_deref(), Some("C0HANDOV1"));
        assert!(slack.app_token.starts_with(b"xapp-"));
        assert!(slack.bot_token.starts_with(b"xoxb-"));
        assert!(cfg.whatsapp().is_err(), "wrong-kind accessor refuses");
    }

    #[test]
    fn slack_config_with_loose_perms_refuses() {
        let dir = tempfile::tempdir().unwrap();
        write_secret(dir.path(), "app_token.txt", "xapp-1-A");
        write_secret(dir.path(), "bot_token.txt", "xoxb-2-B");
        let loose = write_config(dir.path(), "channel-slack-acme.json", SLACK_BODY, 0o644);
        assert!(BridgeConfig::load(&loose).is_err());
    }

    #[test]
    fn slack_config_refuses_bad_channel_shapes_and_tokens() {
        let dir = tempfile::tempdir().unwrap();
        write_secret(dir.path(), "app_token.txt", "xapp-1-A");
        write_secret(dir.path(), "bot_token.txt", "xoxb-2-B");

        // Lowercase junk in a mapped channel id → refuse.
        let bad_shape = write_config(
            dir.path(),
            "channel-slack-bad1.json",
            br#"{"domain":"acme","webhook_secret":"s","app_token_path":"app_token.txt",
                 "bot_token_path":"bot_token.txt","mapped_channels":["c0123abcd"]}"#,
            0o600,
        );
        assert!(BridgeConfig::load(&bad_shape).is_err());

        // 17 channel ids (> 16) → refuse.
        let many = format!(
            r#"{{"domain":"acme","webhook_secret":"s","app_token_path":"app_token.txt",
                "bot_token_path":"bot_token.txt","mapped_channels":[{}]"#,
            (0..17)
                .map(|i| format!(r#""C{i:010}""#))
                .collect::<Vec<_>>()
                .join(",")
        );
        let bad_count = write_config(
            dir.path(),
            "channel-slack-bad2.json",
            many.as_bytes(),
            0o600,
        );
        assert!(BridgeConfig::load(&bad_count).is_err());

        // A non-xapp- app token file → refuse.
        write_secret(dir.path(), "wrong_token.txt", "not-a-socket-mode-token");
        let bad_token = write_config(
            dir.path(),
            "channel-slack-bad3.json",
            br#"{"domain":"acme","webhook_secret":"s","app_token_path":"wrong_token.txt",
                 "bot_token_path":"bot_token.txt","mapped_channels":["C0123ABCD"]}"#,
            0o600,
        );
        assert!(BridgeConfig::load(&bad_token).is_err());
    }

    // ── HERALD PIN: teams config missing its Bot Framework fields refuses.
    #[test]
    fn teams_config_missing_bot_fields_refuses() {
        let dir = tempfile::tempdir().unwrap();
        write_secret(dir.path(), "bot_password.txt", "secret-bytes");

        // Missing bot_app_id → refuse.
        let no_app_id = write_config(
            dir.path(),
            "channel-teams-bad1.json",
            br#"{"domain":"acme","webhook_secret":"s","bot_tenant_id":"11111111-1111-1111-1111-111111111111",
                 "bot_password_path":"bot_password.txt","mapped_channels":["19:abc@thread.tacv2"]}"#,
            0o600,
        );
        assert!(BridgeConfig::load(&no_app_id).is_err());

        // Missing mapped_channels → refuse.
        let no_channels = write_config(
            dir.path(),
            "channel-teams-bad2.json",
            br#"{"domain":"acme","webhook_secret":"s","bot_app_id":"00000000-0000-0000-0000-000000000001",
                 "bot_tenant_id":"11111111-1111-1111-1111-111111111111",
                 "bot_password_path":"bot_password.txt"}"#,
            0o600,
        );
        assert!(BridgeConfig::load(&no_channels).is_err());

        // Missing bot_password file → refuse.
        let no_password = write_config(
            dir.path(),
            "channel-teams-bad3.json",
            br#"{"domain":"acme","webhook_secret":"s","bot_app_id":"00000000-0000-0000-0000-000000000001",
                 "bot_tenant_id":"11111111-1111-1111-1111-111111111111",
                 "bot_password_path":"absent.txt","mapped_channels":["19:abc@thread.tacv2"]}"#,
            0o600,
        );
        assert!(BridgeConfig::load(&no_password).is_err());
    }

    #[test]
    fn teams_config_loads_with_conversation_ids() {
        let dir = tempfile::tempdir().unwrap();
        write_secret(dir.path(), "bot_password.txt", "secret-bytes");
        let cfg_path = write_config(dir.path(), "channel-teams-acme.json", TEAMS_BODY, 0o600);
        let cfg = BridgeConfig::load(&cfg_path).expect("good teams config loads");
        assert_eq!(cfg.kind, "teams");
        let teams = cfg.teams().expect("teams sub-config present");
        assert_eq!(
            teams.mapped_channels,
            vec!["19:abc@thread.tacv2".to_string()]
        );
        assert_eq!(teams.handover_channel, None, "optional handover absent");
        assert_eq!(teams.bot_app_id, "00000000-0000-0000-0000-000000000001");
        assert!(cfg.slack().is_err(), "wrong-kind accessor refuses");
    }
}
