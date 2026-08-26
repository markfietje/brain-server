//! The WFM import grammar — pure, dependency-light parsing shared by the
//! server (seam tests) and the `brain wfm-import` CLI via the
//! `#[path] include` pattern (the shared `http.rs` precedent). No axum, no
//! rusqlite: bytes in, typed rows out.
//!
//! The adapter grammar is deliberately tiny — CSV has no quoting and no
//! embedded commas; anything richer goes through the JSON adapter. Refusing
//! silent misparses beats a fragile parser.

/// Version of the WFM interchange schema. Additive-only change policy:
/// fields are added, never removed or renamed; a breaking need means a NEW
/// version constant and a change-log entry in `docs/wfm-seam.md`.
pub const WFM_SCHEMA_VERSION: &str = "wfm/1";

/// One shift row as the seam imports it (pre-validation; storage-side
/// validation still refuses bad windows and double booking).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedShift {
    pub domain: String,
    pub site: String,
    pub tz: String,
    pub start_epoch: i64,
    pub end_epoch: i64,
    pub overlap_minutes: i64,
    pub roster: Vec<String>,
}

/// A skills import row: one principal-to-skill tag. Import NEVER writes the
/// registry directly — each row becomes a `crew_skills_update` proposal that
/// a human approves (the only write path to `principal_skills`).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedSkill {
    pub principal: String,
    pub skill: String,
}

#[derive(Debug)]
pub enum WfmError {
    /// Structural problem with line/context information.
    Invalid(String),
}

impl std::fmt::Display for WfmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WfmError::Invalid(m) => write!(f, "{m}"),
        }
    }
}

fn invalid(context: &str, detail: impl std::fmt::Display) -> WfmError {
    WfmError::Invalid(format!("{context}: {detail}"))
}

fn parse_epoch(context: &str, raw: &str) -> Result<i64, WfmError> {
    raw.trim().parse::<i64>().map_err(|e| invalid(context, e))
}

const SHIFT_HEADER: [&str; 7] = [
    "domain",
    "site",
    "tz",
    "start_epoch",
    "end_epoch",
    "overlap_minutes",
    "roster",
];

fn shift_from_fields(context: &str, fields: &[&str]) -> Result<ImportedShift, WfmError> {
    let get = |i: usize| -> &str { fields.get(i).copied().unwrap_or("").trim() };
    Ok(ImportedShift {
        domain: get(0).to_string(),
        site: get(1).to_string(),
        tz: if get(2).is_empty() {
            "UTC".to_string()
        } else {
            get(2).to_string()
        },
        start_epoch: parse_epoch(context, get(3))?,
        end_epoch: parse_epoch(context, get(4))?,
        overlap_minutes: if get(5).is_empty() {
            0
        } else {
            parse_epoch(context, get(5))?
        },
        roster: if get(6).is_empty() {
            Vec::new()
        } else {
            get(6)
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        },
    })
}

fn split_csv_line(line: &str) -> Vec<&str> {
    line.split(',').collect()
}

fn check_header(context: &str, header: &[&str], expected: &[&str]) -> Result<(), WfmError> {
    let norm: Vec<String> = header
        .iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .collect();
    if norm != expected.iter().map(|h| h.to_string()).collect::<Vec<_>>() {
        return Err(invalid(
            context,
            format!("header must be exactly `{}`", expected.join(",")),
        ));
    }
    Ok(())
}

/// Parse the generic shifts CSV adapter with a required header row; roster
/// ids are `;`-separated. Bounds and calendar validation stay storage-side.
pub fn parse_shifts_csv(text: &str) -> Result<Vec<ImportedShift>, WfmError> {
    let mut lines = text.lines().enumerate();
    let (_, header) = lines
        .next()
        .ok_or_else(|| WfmError::Invalid("empty csv".into()))?;
    check_header("shifts header", &split_csv_line(header), &SHIFT_HEADER)?;
    let mut out = Vec::new();
    for (i, line) in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields = split_csv_line(line);
        if fields.len() != SHIFT_HEADER.len() {
            return Err(invalid(
                &format!("shifts line {}", i + 1),
                format!(
                    "expected {} fields, got {}",
                    SHIFT_HEADER.len(),
                    fields.len()
                ),
            ));
        }
        out.push(shift_from_fields(
            &format!("shifts line {}", i + 1),
            &fields,
        )?);
    }
    Ok(out)
}

/// Parse the generic shifts JSON adapter: an array of objects with the same
/// fields as the CSV header (tz/overlap/roster optional, same defaults).
pub fn parse_shifts_json(text: &str) -> Result<Vec<ImportedShift>, WfmError> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(text).map_err(|e| invalid("shifts json", e))?;
    let mut out = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let context = format!("shifts json row {}", i);
        let req = |key: &str| -> Result<String, WfmError> {
            row.get(key)
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| invalid(&context, format!("missing string field `{key}`")))
        };
        let num = |key: &str| -> Result<i64, WfmError> {
            row.get(key)
                .and_then(|v| v.as_i64())
                .ok_or_else(|| invalid(&context, format!("missing integer field `{key}`")))
        };
        out.push(ImportedShift {
            domain: req("domain")?,
            site: req("site")?,
            tz: row
                .get("tz")
                .and_then(|v| v.as_str())
                .unwrap_or("UTC")
                .to_string(),
            start_epoch: num("start_epoch")?,
            end_epoch: num("end_epoch")?,
            overlap_minutes: row
                .get("overlap_minutes")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            roster: match row.get("roster") {
                None | Some(serde_json::Value::Null) => Vec::new(),
                Some(v) => serde_json::from_value(v.clone())
                    .map_err(|e| invalid(&context, format!("roster: {e}")))?,
            },
        });
    }
    Ok(out)
}

const SKILLS_HEADER: [&str; 2] = ["principal", "skill"];

fn skills_from_fields(context: &str, fields: &[&str]) -> Result<ImportedSkill, WfmError> {
    let get = |i: usize| -> &str { fields.get(i).copied().unwrap_or("").trim() };
    let principal = get(0);
    let skill = get(1);
    if principal.is_empty() || skill.is_empty() {
        return Err(invalid(context, "principal and skill are both required"));
    }
    Ok(ImportedSkill {
        principal: principal.to_string(),
        skill: skill.to_string(),
    })
}

/// Parse the generic skills CSV adapter: `principal,skill` rows (a strict
/// header is preferred; a headerless two-column export also round-trips).
pub fn parse_skills_csv(text: &str) -> Result<Vec<ImportedSkill>, WfmError> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let fields = split_csv_line(line);
        if index == 0
            && fields.len() == SKILLS_HEADER.len()
            && check_header("skills header", &fields, &SKILLS_HEADER).is_ok()
        {
            continue;
        }
        if fields.len() != SKILLS_HEADER.len() {
            return Err(invalid(
                &format!("skills line {}", index + 1),
                format!(
                    "expected {} fields, got {}",
                    SKILLS_HEADER.len(),
                    fields.len()
                ),
            ));
        }
        out.push(skills_from_fields(
            &format!("skills line {}", index + 1),
            &fields,
        )?);
    }
    Ok(out)
}

/// Parse the generic skills JSON adapter:
/// `[{"principal": "...", "skill": "..."}]`.
pub fn parse_skills_json(text: &str) -> Result<Vec<ImportedSkill>, WfmError> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(text).map_err(|e| invalid("skills json", e))?;
    let mut out = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let context = format!("skills json row {}", i);
        let req = |key: &str| -> Result<String, WfmError> {
            row.get(key)
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| invalid(&context, format!("missing string field `{key}`")))
        };
        out.push(ImportedSkill {
            principal: req("principal")?,
            skill: req("skill")?,
        });
    }
    Ok(out)
}
