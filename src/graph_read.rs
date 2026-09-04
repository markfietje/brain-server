//! Graph read helpers: the `?limit=` clamp and the traversal-row mappers
//! behind the graph surfaces. Take connections and plain rows — no
//! transport types; the graph handlers call in from `main.rs`.

use crate::config::MAX_GRAPH_EDGES;

/// Clamp a graph `?limit=` into `1..=MAX_GRAPH_EDGES` (a missing or bogus value
/// falls back to the default cap). Shared by `get_entity` and `get_relations`.
pub fn clamp_graph_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(MAX_GRAPH_EDGES).clamp(1, MAX_GRAPH_EDGES)
}

/// row mapper for the recursive CTE. Extracted so all four
/// param-shape branches share one definition (DRY; the only thing that varies
/// is which params are bound, not how the row maps).
pub(crate) fn traverse_row_mapper(
    domain: &str,
) -> impl Fn(&rusqlite::Row<'_>) -> rusqlite::Result<serde_json::Value> + '_ {
    move |r| {
        Ok(serde_json::json!({
            "entity": r.get::<_, String>(0)?,
            "depth": r.get::<_, i64>(1)?,
            "path": r.get::<_, String>(2)?,
            "edge_path": r.get::<_, String>(3)?,
            "from_entity": r.get::<_, Option<String>>(4)?,
            "domain": domain,
        }))
    }
}

/// turn the flat traversal rows into structured hop chains.
/// Each row's `path` is `id->id->id` and `edge_path` is `rel|rel|rel`. We pair
/// them with the entity names already on the row (the leaf) and the from_entity
/// (the seed) to reconstruct the named chain. ponytail: this is a best-effort
/// reconstruction from the CTE output; a true path-aware walk would carry
/// (entity, rel) tuples through the recursion. That's a larger change; this is
/// the smallest faithful explanation that reuses the existing bounded BFS and
/// stays inside MAX_VISITED. Intermediate node names are NOT resolved here —
/// hops surface the seed name, the leaf name, and every id; a consuming agent
/// that needs an intermediate's name calls `/get/{id}` on the id.
pub(crate) fn build_explanation_paths(rows: &[serde_json::Value]) -> Vec<serde_json::Value> {
    if rows.is_empty() {
        return Vec::new();
    }
    rows.iter()
        .map(|row| {
            let path_str = row.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let edge_str = row.get("edge_path").and_then(|v| v.as_str()).unwrap_or("");
            let ids: Vec<&str> = path_str.split("->").filter(|s| !s.is_empty()).collect();
            let rels: Vec<&str> = edge_str.split('|').filter(|s| !s.is_empty()).collect();
            // Build the hop chain. ids.len() == rels.len()+1 (one more node than
            // edges); zip them so each hop is {from, relation, to}. The first
            // node's name is `from_entity`; the last is `entity`.
            let leaf = row.get("entity").and_then(|v| v.as_str()).unwrap_or("");
            let seed = row
                .get("from_entity")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mut hops: Vec<serde_json::Value> = Vec::new();
            for (i, rel) in rels.iter().enumerate() {
                let from_id = ids.get(i).copied().unwrap_or("");
                let to_id = ids.get(i + 1).copied().unwrap_or("");
                // First hop's from is the named seed; last hop's to is the named leaf.
                let from_name = if i == 0 { seed } else { "" };
                let to_name = if i + 1 == rels.len() { leaf } else { "" };
                hops.push(serde_json::json!({
                    "from": {"id": from_id, "name": from_name},
                    "relation": rel,
                    "to": {"id": to_id, "name": to_name},
                }));
            }
            serde_json::json!({
                "hops": hops,
                "depth": row.get("depth").cloned().unwrap_or(serde_json::Value::Null),
                "domain": row.get("domain").cloned().unwrap_or(serde_json::Value::Null),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explanation_paths_reconstruct_hop_chain_from_cte_output() {
        // build_explanation_paths must turn a flat traversal
        // row (path="1->5->9", edge_path="works_at|ceo_of") into a structured
        // hop chain with named endpoints. This is the faithful explanation
        // the roadmap exit criterion asks for.
        let rows = vec![serde_json::json!({
            "entity": "acme_corp",
            "depth": 2,
            "path": "1->5->9",
            "edge_path": "works_at|ceo_of",
            "from_entity": "alice",
            "domain": "global"
        })];
        let paths = build_explanation_paths(&rows);
        assert_eq!(paths.len(), 1);
        let hops = paths[0]["hops"].as_array().unwrap();
        assert_eq!(hops.len(), 2, "two edges → two hops");
        // First hop: seed (named) → intermediate (id only).
        assert_eq!(hops[0]["from"]["name"].as_str().unwrap(), "alice");
        assert_eq!(hops[0]["relation"].as_str().unwrap(), "works_at");
        assert_eq!(hops[0]["to"]["id"].as_str().unwrap(), "5");
        // Second hop: intermediate (id only) → leaf (named).
        assert_eq!(hops[1]["from"]["id"].as_str().unwrap(), "5");
        assert_eq!(hops[1]["relation"].as_str().unwrap(), "ceo_of");
        assert_eq!(hops[1]["to"]["name"].as_str().unwrap(), "acme_corp");
    }

    #[test]
    fn explanation_paths_empty_on_empty_input() {
        // No traversal rows → no paths. The consuming agent sees `paths: []`.
        assert!(build_explanation_paths(&[]).is_empty());
    }
}
