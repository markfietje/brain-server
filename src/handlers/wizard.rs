//! The wizard pack catalog's read surface: the three operator-ratified
//! packs as compile-time-embedded, validated DATA — the SvelteTauri shell's
//! one declared kernel need (the typed-wire skeleton's pack read). Protocol
//! adapter only, and less: the catalog is PURE data — no State, no pool, no
//! db, no spawn_blocking, and this file carries ZERO SQL statements (the
//! no-SQL-in-handlers law counts it live at zero).
//!
//! Gate posture: `Read` on the global domain — the ratified posture (the
//! pack templates are PII-free schema data; any authenticated principal may
//! read them). No role gate (reading a template is not calibrating), no
//! PRE_GATE (no `{id}` in the path), and the catalog is never empty so it
//! joins neither the probe-blind nor the empty-safe lists.

use axum::Json;

use crate::handlers::auth::OptPrincipal;
use crate::handlers::{HandlerError, authorize};

/// `GET /workflow/wizard/packs` — the validated ratified pack catalog:
/// `{packs: [{id, question_count, pack}], count}` where `pack` is the pack
/// JSON verbatim (the committed corpus bytes), re-validated through the
/// shipped total validator at read time.
pub async fn get_wizard_packs(
    principal: OptPrincipal,
) -> Result<Json<serde_json::Value>, HandlerError> {
    authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    Ok(Json(wizard_packs_payload()))
}

/// The pure, principal-independent payload — the gate-free purity the unit
/// test pins: no I/O, no db, total, and every served entry validated.
fn wizard_packs_payload() -> serde_json::Value {
    let catalog = crate::workflow::wizard::wizard_pack_catalog();
    let count = catalog.len();
    let packs: Vec<serde_json::Value> = catalog
        .into_iter()
        .map(|entry| {
            serde_json::json!({
                "id": entry.id,
                "question_count": entry.question_count,
                "pack": entry.pack,
            })
        })
        .collect();
    serde_json::json!({ "packs": packs, "count": count })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// wizard_packs_payload_carries_the_validated_ratified_packs — the
    /// payload carries EXACTLY the ratified id set in order, the shape is
    /// closed (id + question_count + pack), and every served pack
    /// re-validates through the shipped total validator. The handler adds
    /// only the Read gate on top of this pure value.
    #[test]
    fn wizard_packs_payload_carries_the_validated_ratified_packs() {
        let payload = wizard_packs_payload();
        let ids: Vec<&str> = payload["packs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, crate::workflow::wizard::RATIFIED_PACK_IDS.to_vec());
        assert_eq!(
            payload["count"],
            serde_json::json!(crate::workflow::wizard::RATIFIED_PACK_IDS.len())
        );
        for entry in payload["packs"].as_array().unwrap() {
            assert!(entry["question_count"].as_u64().unwrap() >= 1);
            let pack = crate::workflow::wizard::validate_wizard_pack(&entry["pack"])
                .unwrap_or_else(|e| panic!("{}: the served pack re-validates: {e}", entry["id"]));
            assert_eq!(pack.pack, entry["id"].as_str().unwrap().to_string());
        }
    }
}
