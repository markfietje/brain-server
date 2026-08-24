//! Shift tables and the ring view — follow-the-sun as data.
//!
//! A `shifts` row is one site's on-call window: `(site, tz, start, end,
//! overlap_minutes, roster)`. The ring is the domain's shifts ordered by
//! start; "queue follows the sun, cases don't" becomes literal arithmetic:
//! [`ring_view`] computes, for any instant, which site owns the queue
//! (`queue_scope_site`), whether that instant sits inside the overlap window
//! derived from the boundary's shift pair ([`overlap_window`]), and when the
//! next boundary lands. Open runs are never touched — this module reads
//! shift rows only; re-scoping is a property of the view, not a write.
//!
//! Deterministic by construction: pure time-table arithmetic over stored
//! rows, computed at read time; there is no scheduler daemon.

use rusqlite::Connection;

/// Hard cap on a single overlap window (minutes). The research band is
/// 30–120; anything above defeats the point of a boundary.
pub const MAX_OVERLAP_MINUTES: i64 = 120;

#[derive(Debug, Clone, PartialEq)]
pub struct Shift {
    pub id: i64,
    pub domain: String,
    pub site: String,
    pub tz: String,
    pub start_epoch: i64,
    pub end_epoch: i64,
    pub overlap_minutes: i64,
    /// Principal ids on this shift (JSON array at rest).
    pub roster: Vec<String>,
}

#[derive(Debug)]
pub enum ShiftError {
    BadWindow,
    BadOverlap,
    DoubleBooking { with_id: i64 },
    InvalidRoster(String),
    Database(String),
}

impl std::fmt::Display for ShiftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShiftError::BadWindow => write!(f, "shift window invalid (end must be after start)"),
            ShiftError::BadOverlap => {
                write!(f, "overlap_minutes must be 0..={MAX_OVERLAP_MINUTES}")
            }
            ShiftError::DoubleBooking { with_id } => {
                write!(f, "shift overlaps existing shift id {with_id}")
            }
            ShiftError::InvalidRoster(e) => write!(f, "roster is not a JSON string array: {e}"),
            ShiftError::Database(e) => write!(f, "database error: {e}"),
        }
    }
}

fn db_err(e: rusqlite::Error) -> ShiftError {
    ShiftError::Database(e.to_string())
}

fn overlap_secs(s: &Shift) -> i64 {
    s.overlap_minutes * 60
}

/// Validate a candidate window/overlap pair independent of stored rows.
pub fn validate_window(
    start_epoch: i64,
    end_epoch: i64,
    overlap_minutes: i64,
) -> Result<(), ShiftError> {
    if end_epoch <= start_epoch {
        return Err(ShiftError::BadWindow);
    }
    if !(0..=MAX_OVERLAP_MINUTES).contains(&overlap_minutes) {
        return Err(ShiftError::BadOverlap);
    }
    Ok(())
}

/// The overlap window a boundary derives from its shift pair: the first
/// `prev.overlap_minutes` of `cur`'s window, clipped so it can never run
/// past `prev.end`. `None` when the pair declares no overlap or does not
/// actually share time.
pub fn overlap_window(prev: &Shift, cur: &Shift) -> Option<(i64, i64)> {
    let start = cur.start_epoch;
    let end = (cur.start_epoch + overlap_secs(prev)).min(prev.end_epoch);
    if prev.overlap_minutes > 0 && end > start && start >= prev.start_epoch {
        Some((start, end))
    } else {
        None
    }
}

/// No double booking: within one domain two shifts may share time only when
/// the shared span fits entirely inside the earlier shift's declared overlap
/// window.
pub fn check_no_double_booking(
    existing: &[Shift],
    domain: &str,
    start_epoch: i64,
    end_epoch: i64,
    overlap_minutes: i64,
) -> Result<(), ShiftError> {
    validate_window(start_epoch, end_epoch, overlap_minutes)?;
    // Pair the candidate with each same-domain shift it shares time with.
    // An (earlier e, later l) pair may share only e's final overlap period:
    // l must START no earlier than e.end − e.overlap (exactly where
    // `overlap_window` derives the read-time boundary).
    for ex in existing.iter().filter(|s| s.domain == domain) {
        let inter = (end_epoch.min(ex.end_epoch)).saturating_sub(start_epoch.max(ex.start_epoch));
        if inter == 0 {
            continue;
        }
        let candidate_is_earlier = start_epoch <= ex.start_epoch;
        let (e_start, e_end, e_overlap, l_start) = if candidate_is_earlier {
            (start_epoch, end_epoch, overlap_minutes, ex.start_epoch)
        } else {
            (
                ex.start_epoch,
                ex.end_epoch,
                ex.overlap_minutes,
                start_epoch,
            )
        };
        let earliest_shared_start = (e_end - e_overlap * 60).max(e_start);
        if l_start < earliest_shared_start {
            return Err(ShiftError::DoubleBooking { with_id: ex.id });
        }
    }
    Ok(())
}

/// The shift whose window contains `now` (`start <= now < end`), latest
/// start wins.
pub fn active_shift<'a>(shifts: &'a [Shift], domain: &str, now: i64) -> Option<&'a Shift> {
    shifts
        .iter()
        .filter(|s| s.domain == domain && s.start_epoch <= now && now < s.end_epoch)
        .max_by_key(|s| s.start_epoch)
}

/// The shift immediately before `cur` in the same domain (by start order),
/// regardless of adjacency.
fn previous_shift<'a>(shifts: &'a [Shift], domain: &str, cur: &Shift) -> Option<&'a Shift> {
    shifts
        .iter()
        .filter(|s| s.domain == domain && s.id != cur.id && s.start_epoch <= cur.start_epoch)
        .max_by_key(|s| s.start_epoch)
}

#[derive(Debug, Clone, PartialEq)]
pub struct RingView {
    pub now: i64,
    pub domain: String,
    /// Site that owns the queue at `now` (None before the first shift).
    pub queue_scope_site: Option<String>,
    /// The incoming site while inside an overlap window.
    pub incoming_site: Option<String>,
    pub in_overlap: bool,
    /// Epoch of the next hard boundary (end of the active shift, or of its
    /// overlap window when one is running).
    pub next_boundary_epoch: Option<i64>,
}

/// Queue follows the sun, cases don't: the queue re-scopes to the incoming
/// site at the START of the derived overlap window (not at the hard
/// boundary); open runs are never consulted or mutated here.
pub fn ring_view(shifts: &[Shift], domain: &str, now: i64) -> RingView {
    let current = active_shift(shifts, domain, now);
    let mut view = RingView {
        now,
        domain: domain.to_string(),
        queue_scope_site: current.map(|s| s.site.clone()),
        incoming_site: None,
        in_overlap: false,
        next_boundary_epoch: current.map(|s| s.end_epoch),
    };
    if let Some(cur) = current
        && let Some(prev) = previous_shift(shifts, domain, cur)
        && let Some((ov_start, ov_end)) = overlap_window(prev, cur)
        && now >= ov_start
        && now < ov_end
    {
        view.in_overlap = true;
        view.incoming_site = Some(cur.site.clone());
        view.queue_scope_site = Some(cur.site.clone());
        view.next_boundary_epoch = Some(ov_end);
    }
    view
}

fn roster_from(json: &str) -> Result<Vec<String>, ShiftError> {
    serde_json::from_str::<Vec<String>>(json).map_err(|e| ShiftError::InvalidRoster(e.to_string()))
}

/// Read cap for list surfaces — the newest `limit` shifts (by start), returned
/// ascending. The ring arithmetic only needs the current schedule window.
pub const MAX_SHIFTS_RETURNED: i64 = 500;

/// All shifts for a domain (newest-capped at [`MAX_SHIFTS_RETURNED`]), ordered
/// by start. A corrupt roster cell fails closed (the whole read errors, never
/// an empty-roster silent degrade).
pub fn list_shifts(conn: &Connection, domain: &str) -> Result<Vec<Shift>, ShiftError> {
    let sql = "SELECT id, domain, site, tz, start_epoch, end_epoch, roster_json, overlap_minutes
               FROM shifts WHERE domain = ?1
               ORDER BY start_epoch DESC LIMIT ?2";
    let mut stmt = conn.prepare(sql).map_err(db_err)?;
    let rows = stmt
        .query_map(rusqlite::params![domain, MAX_SHIFTS_RETURNED], |r| {
            let roster_json: String = r.get(6)?;
            Ok(Shift {
                id: r.get(0)?,
                domain: r.get(1)?,
                site: r.get(2)?,
                tz: r.get(3)?,
                start_epoch: r.get(4)?,
                end_epoch: r.get(5)?,
                overlap_minutes: r.get(7)?,
                roster: roster_from(&roster_json).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        6,
                        rusqlite::types::Type::Text,
                        e.to_string().into(),
                    )
                })?,
            })
        })
        .map_err(db_err)?;
    let mut out: Vec<Shift> = Vec::new();
    for s in rows {
        out.push(s.map_err(db_err)?);
    }
    out.reverse();
    Ok(out)
}

/// A shift candidate for storage (pre-id).
#[derive(Debug, Clone)]
pub struct ShiftDraft<'a> {
    pub domain: &'a str,
    pub site: &'a str,
    pub tz: &'a str,
    pub start_epoch: i64,
    pub end_epoch: i64,
    pub overlap_minutes: i64,
    pub roster: &'a [String],
}

/// Insert a validated shift; refuses double booking against stored rows in
/// the same domain. Caller provides the tx (the audit row rides the same tx).
pub fn insert_shift(conn: &Connection, draft: &ShiftDraft<'_>) -> Result<i64, ShiftError> {
    let ShiftDraft {
        domain,
        site,
        tz,
        start_epoch,
        end_epoch,
        overlap_minutes,
        roster,
    } = draft;
    check_no_double_booking(
        &list_shifts(conn, domain)?,
        domain,
        *start_epoch,
        *end_epoch,
        *overlap_minutes,
    )?;
    let roster_json =
        serde_json::to_string(roster).map_err(|e| ShiftError::InvalidRoster(e.to_string()))?;
    conn.execute(
        "INSERT INTO shifts(domain, site, tz, start_epoch, end_epoch, overlap_minutes, roster_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            domain,
            site,
            tz,
            start_epoch,
            end_epoch,
            overlap_minutes,
            roster_json,
            chrono::Utc::now().timestamp()
        ],
    )
    .map_err(db_err)?;
    Ok(conn.last_insert_rowid())
}

#[cfg(test)]
fn draft<'a>(domain: &'a str, site: &'a str, start: i64, end: i64, ov: i64) -> ShiftDraft<'a> {
    ShiftDraft {
        domain,
        site,
        tz: "UTC",
        start_epoch: start,
        end_epoch: end,
        overlap_minutes: ov,
        roster: &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;

    fn seed() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("open");
        run_migration(&mut conn, 1).expect("migration");
        conn
    }

    fn shift(id: i64, site: &str, start: i64, end: i64, ov: i64) -> Shift {
        Shift {
            id,
            domain: "global".into(),
            site: site.into(),
            tz: "UTC".into(),
            start_epoch: start,
            end_epoch: end,
            overlap_minutes: ov,
            roster: vec!["alice".into()],
        }
    }

    #[test]
    fn overlap_window_derives_from_shift_pair() {
        // Manila 08:00–16:00 declaring a 60-min overlap; Amsterdam starts 15:00.
        let manila = shift(1, "manila", 28_800, 57_600, 60);
        let ams = shift(2, "amsterdam", 54_000, 82_800, 0);
        let w = overlap_window(&manila, &ams).expect("overlap derives");
        assert_eq!(w, (54_000, 57_600));
        // Clipped to the outgoing shift's end when the declaration overruns it.
        let manila_short = shift(3, "manila", 28_800, 55_200, 120);
        assert_eq!(overlap_window(&manila_short, &ams), Some((54_000, 55_200)));
        // Zero declaration → no window; reversed pair → no window;
        // non-touching pair → no window.
        let manila_plain = shift(5, "manila", 28_800, 57_600, 0);
        assert_eq!(overlap_window(&manila_plain, &ams), None);
        assert_eq!(overlap_window(&ams, &manila), None);
        let far = shift(4, "tokyo", 500_000, 600_000, 30);
        assert_eq!(overlap_window(&manila, &far), None);
        // Storage round-trip preserves the fields the derivation reads.
        let conn = seed();
        let roster = ["a".to_string()];
        let id = insert_shift(
            &conn,
            &ShiftDraft {
                domain: "global",
                site: "manila",
                tz: "+08:00",
                start_epoch: 800,
                end_epoch: 1600,
                overlap_minutes: 60,
                roster: &roster,
            },
        )
        .expect("insert");
        let listed = list_shifts(&conn, "global").expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].roster, vec!["a".to_string()]);
    }

    #[test]
    fn shift_table_validates_no_double_booking() {
        let conn = seed();
        // Manila owns 08:00–16:00 and declares a 60-min handover budget.
        insert_shift(&conn, &draft("global", "manila", 28_800, 57_600, 60)).expect("first shift");
        // Fully inside another shift → refused.
        let err = insert_shift(&conn, &draft("global", "rogue", 36_000, 40_000, 0)).unwrap_err();
        assert!(matches!(err, ShiftError::DoubleBooking { .. }));
        // Sharing time OUTSIDE the declared overlap budget (starts more than
        // an hour before Manila ends) → refused.
        let err = insert_shift(&conn, &draft("global", "ams", 50_000, 82_800, 0)).unwrap_err();
        assert!(matches!(err, ShiftError::DoubleBooking { .. }));
        // A window inside the declared overlap budget → accepted.
        insert_shift(&conn, &draft("global", "ams", 54_000, 82_800, 0))
            .expect("declared overlap ok");
        // Bad windows refuse before any booking check.
        assert!(matches!(
            validate_window(2000, 2000, 0),
            Err(ShiftError::BadWindow)
        ));
        assert!(matches!(
            validate_window(2000, 3000, MAX_OVERLAP_MINUTES + 1),
            Err(ShiftError::BadOverlap)
        ));
        // Other domains never collide.
        insert_shift(&conn, &draft("acme", "other", 28_800, 57_600, 0)).expect("cross-domain ok");
    }

    #[test]
    fn ring_boundary_rescopes_queue_not_cases() {
        let shifts = vec![
            shift(1, "manila", 1000, 2000, 60),
            shift(2, "amsterdam", 1940, 3000, 0),
        ];
        // Before the boundary: Manila owns the queue.
        let v = ring_view(&shifts, "global", 1500);
        assert_eq!(v.queue_scope_site.as_deref(), Some("manila"));
        assert!(!v.in_overlap);
        assert_eq!(v.next_boundary_epoch, Some(2000));
        // Inside the derived overlap window: the queue ALREADY follows the
        // sun to the incoming site.
        let v = ring_view(&shifts, "global", 1950);
        assert!(v.in_overlap);
        assert_eq!(v.queue_scope_site.as_deref(), Some("amsterdam"));
        assert_eq!(v.incoming_site.as_deref(), Some("amsterdam"));
        assert_eq!(v.next_boundary_epoch, Some(2000));
        // After the outgoing shift ends: still Amsterdam, overlap flag clear.
        let v = ring_view(&shifts, "global", 2100);
        assert!(!v.in_overlap);
        assert_eq!(v.queue_scope_site.as_deref(), Some("amsterdam"));
        // Before any shift: no scope.
        assert_eq!(ring_view(&shifts, "global", 500).queue_scope_site, None);
        // Cases don't follow: an open run row is untouched by the re-scope —
        // the view reads shift rows only, proven by mutating nothing here and
        // asserting the run row survives byte-identical across a boundary.
        let conn = seed();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('global', 'interview', '{\"owner\":\"manila\"}', 0, 'active', 1000, 1000)",
            [],
        )
        .expect("seed run");
        let before: String = conn
            .query_row(
                "SELECT state_json FROM workflow_runs WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .expect("read");
        let _ = ring_view(&shifts, "global", 1950);
        let after: String = conn
            .query_row(
                "SELECT state_json FROM workflow_runs WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .expect("read");
        assert_eq!(before, after);
        let rev: i64 = conn
            .query_row(
                "SELECT state_revision FROM workflow_runs WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .expect("read");
        assert_eq!(rev, 0);
    }
}
