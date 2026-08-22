//! Capability gating for hostcalls (the kernel-level half of 1.28.2's
//! full policy): every dispatch checks a posture, unknown operation classes
//! are denied, and denials are loud values rather than silent fallbacks.

use crate::host::{AuditKind, AuditStatus, WorkflowHost};
use std::sync::Arc;

/// Trust postures, coarsest first. `Safe` exposes only pure operations;
/// `Permissive` is for local, non-privileged development.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Safe,
    Standard,
    Permissive,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::Safe => "safe",
            Capability::Standard => "standard",
            Capability::Permissive => "permissive",
        }
    }
}

/// Hostcall operation classes the kernel knows about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpClass {
    ReadState,
    WriteState,
    ExecTool,
    FilesystemWrite,
    ProcessSpawn,
}

impl OpClass {
    pub fn as_str(self) -> &'static str {
        match self {
            OpClass::ReadState => "read_state",
            OpClass::WriteState => "write_state",
            OpClass::ExecTool => "exec_tool",
            OpClass::FilesystemWrite => "fs_write",
            OpClass::ProcessSpawn => "process_spawn",
        }
    }
}

/// Fail-closed gate: any (posture, class) pair this kernel does not
/// explicitly allow is denied. The full per-engine deny/allow policy lands
/// with the trust release; this is the invariant it must preserve.
pub const fn allows(posture: Capability, class: OpClass) -> bool {
    matches!(
        (posture, class),
        (Capability::Safe, OpClass::ReadState)
            | (
                Capability::Standard,
                OpClass::ReadState | OpClass::WriteState | OpClass::ExecTool
            )
            | (
                Capability::Permissive,
                OpClass::ReadState
                    | OpClass::WriteState
                    | OpClass::ExecTool
                    | OpClass::FilesystemWrite
                    | OpClass::ProcessSpawn
            )
    )
}

/// Check-and-audit in one step: the decision and its evidence row are
/// produced together, so a denial can never bypass the chain silently.
pub fn checked_dispatch<H: WorkflowHost + ?Sized>(
    host: &Arc<H>,
    posture: Capability,
    class: OpClass,
    actor: &str,
) -> bool {
    let allowed = allows(posture, class);
    host.audit(
        AuditKind::Workflow,
        actor,
        class.as_str(),
        if allowed {
            AuditStatus::Ok
        } else {
            AuditStatus::Denied
        },
        posture.as_str(),
    );
    allowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{AuditKind, AuditStatus, CasError, HostError, HostTx};
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct TapeHost {
        log: StdMutex<Vec<String>>,
    }
    impl WorkflowHost for TapeHost {
        fn tx(&self) -> Result<HostTx, HostError> {
            unreachable!("not exercised here")
        }
        fn enqueue(&self, _: i64, _: &str, _: &str, _: &str) -> Result<bool, HostError> {
            Ok(true)
        }
        fn cas(&self, _: i64, _: i64, _: &str) -> Result<(), CasError> {
            Ok(())
        }
        fn load_state(&self, _: i64) -> Result<Option<(String, i64)>, HostError> {
            Ok(None)
        }
        fn audit(&self, k: AuditKind, actor: &str, target: &str, s: AuditStatus, d: &str) {
            if let Ok(mut g) = self.log.lock() {
                g.push(format!(
                    "{}/{}/{}/{}/{}",
                    k.as_str(),
                    actor,
                    target,
                    s.as_str(),
                    d
                ));
            }
        }
    }

    #[test]
    fn posture_ladder_is_monotonic_and_fail_closed() {
        // Safe ⊂ Standard ⊂ Permissive.
        assert!(allows(Capability::Safe, OpClass::ReadState));
        assert!(!allows(Capability::Safe, OpClass::ExecTool));
        assert!(allows(Capability::Standard, OpClass::WriteState));
        assert!(!allows(Capability::Standard, OpClass::ProcessSpawn));
        for class in [
            OpClass::ReadState,
            OpClass::WriteState,
            OpClass::ExecTool,
            OpClass::FilesystemWrite,
            OpClass::ProcessSpawn,
        ] {
            assert!(allows(Capability::Permissive, class));
        }
        // Vocabulary strings stay stable (wire-visible).
        assert_eq!(Capability::Standard.as_str(), "standard");
        assert_eq!(OpClass::FilesystemWrite.as_str(), "fs_write");
    }

    #[test]
    fn checked_dispatch_audits_denials_too() {
        let host = Arc::new(TapeHost::default());
        let ok = checked_dispatch(&host, Capability::Safe, OpClass::ReadState, "engine-a");
        let denied = checked_dispatch(&host, Capability::Safe, OpClass::ProcessSpawn, "engine-a");
        assert!(ok);
        assert!(!denied);
        let log = host.log.lock().map(|g| g.clone()).unwrap_or_default();
        assert_eq!(
            log,
            vec![
                "workflow/engine-a/read_state/ok/safe",
                "workflow/engine-a/process_spawn/denied/safe"
            ]
        );
    }
}
