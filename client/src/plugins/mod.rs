//! The Cordis-shaped plugin kernel: the shell, the chat surface, and the HITL
//! control panel are separate plugins composed through slots — not one
//! monolithic app. Every mountable UI region is registered, never imported.
//!
//! Contract (ported semantics; original Rust — compile-time plugins only):
//! - **One API:** a plugin composes only through [`PluginCtx::register`]; the
//!   shell alone owns the `root` family.
//! - **Declaration = authorization:** registration into an undeclared slot
//!   family is a load error (the conflict is the design speaking). Slot names
//!   mirror the composition path `<domain>.<entry>.<hole>`.
//! - **Double-declare fails loud:** a family or slot key already owned by
//!   another live plugin is a mount error, never a silent overwrite.
//! - **Domains share only JSON-compatible data** (`SlotSpec.store`) + the
//!   registry itself; cross-plugin value imports are build errors.
//! - **Unload reverses:** unmounting a plugin removes exactly its
//!   registrations and bumps the registry revision (the `slots/changed`
//!   payload) — hot-reload swaps registrations, never running fibers.

pub mod chat;
pub mod control_panel;
pub mod shell;

use crate::slots::{SlotKind, SlotRegistry, SlotSpec};

/// The ctx keys each built-in domain publishes (services share data through
/// these, never through direct imports).
///
/// Truthful allow (the slots.rs precedent): the service-identity keys are the
/// published contract of each plugin; their runtime readers (cross-plugin
/// service lookup) land with the streaming conversation surface — exercised by
/// the plugin tests today.
#[allow(dead_code)]
pub const CTX_LAYOUT: &str = "ctx.layout";
#[allow(dead_code)]
pub const CTX_CONVERSATION: &str = "ctx.conversation";
#[allow(dead_code)]
pub const CTX_APPROVALS: &str = "ctx.approvals";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    /// A family or slot key is already owned by another live plugin.
    Conflict { owner: &'static str, detail: String },
    /// Registration into a family this plugin did not declare.
    Undeclared {
        owner: &'static str,
        family: &'static str,
    },
    /// The same plugin mounted twice (hot reload must unmount first).
    DoubleMount { name: &'static str },
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict { owner, detail } => write!(f, "plugin `{owner}` conflict: {detail}"),
            Self::Undeclared { owner, family } => {
                write!(
                    f,
                    "plugin `{owner}` registered into undeclared family `{family}`"
                )
            }
            Self::DoubleMount { name } => write!(f, "plugin `{name}` already mounted"),
        }
    }
}

impl std::error::Error for PluginError {}

/// One UI plugin. `declares` is the authorization set: only these slot
/// families accept this plugin's registrations.
pub trait Plugin {
    fn name(&self) -> &'static str;
    /// The ctx service key this plugin publishes (`ctx.layout`, …).
    /// Truthful allow (see CTX_*): read by tests today; runtime service
    /// lookup lands with the streaming conversation surface.
    #[allow(dead_code)]
    fn ctx_key(&self) -> &'static str;
    /// Slot families owned by this plugin (declaration = authorization).
    fn declares(&self) -> &'static [&'static str];
    /// Register this plugin's slot entries. Fails loud on any conflict.
    fn mount(&self, ctx: &mut PluginCtx) -> Result<(), PluginError>;
}

#[derive(Clone)]
struct Mounted {
    name: &'static str,
    declared: &'static [&'static str],
    applied: Vec<(&'static str, String)>,
}

/// The composition host: mounts/unmounts plugins over one shared
/// [`SlotRegistry`] and tracks ownership so unload reverses exactly.
#[derive(Default, Clone)]
pub struct PluginHost {
    pub slots: SlotRegistry,
    mounted: Vec<Mounted>,
}

impl PluginHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Boot composition: shell → chat → control panel, in order. A failure at
    /// any step aborts the whole boot (fail loud — no partial shell).
    pub fn boot() -> Result<Self, PluginError> {
        let mut host = Self::new();
        host.mount(&shell::Shell)?;
        host.mount(&chat::Chat)?;
        host.mount(&control_panel::ControlPanel)?;
        Ok(host)
    }

    fn owner_of(&self, family: &str) -> Option<&'static str> {
        self.mounted
            .iter()
            .find(|m| m.declared.contains(&family))
            .map(|m| m.name)
    }

    /// Mount one plugin. Double-declare (family or key owned elsewhere) and
    /// undeclared-family registrations fail loud; a mid-mount failure rolls
    /// back the entries that plugin already applied.
    pub fn mount<P: Plugin>(&mut self, plugin: &P) -> Result<(), PluginError> {
        let name = plugin.name();
        if self.mounted.iter().any(|m| m.name == name) {
            return Err(PluginError::DoubleMount { name });
        }
        for family in plugin.declares() {
            if let Some(prev) = self.owner_of(family) {
                return Err(PluginError::Conflict {
                    owner: name,
                    detail: format!("family `{family}` already owned by `{prev}`"),
                });
            }
        }
        // The known-family set: everything declared by live plugins plus this
        // one — a registration into a family nobody declared fails loud.
        let mut known: Vec<&'static str> = self
            .mounted
            .iter()
            .flat_map(|m| m.declared.iter().copied())
            .collect();
        known.extend(plugin.declares().iter().copied());
        let mut ctx = PluginCtx {
            slots: &mut self.slots,
            owner: name,
            known,
            applied: Vec::new(),
            owners: self
                .mounted
                .iter()
                .flat_map(|m| m.applied.iter().map(|(f, k)| (*f, k.clone(), m.name)))
                .collect(),
        };
        let res = plugin.mount(&mut ctx);
        match res {
            Ok(()) => {
                self.mounted.push(Mounted {
                    name,
                    declared: plugin.declares(),
                    applied: ctx.applied,
                });
                Ok(())
            }
            Err(e) => {
                // Roll back this plugin's partial registrations; the host stays
                // at its pre-mount shape (revision still moved — observers see
                // the failed swap too).
                for (family, key) in &ctx.applied {
                    self.slots.remove_in(family, key);
                }
                Err(e)
            }
        }
    }

    /// Unmount by name: removes exactly this plugin's registrations and bumps
    /// the revision (the `slots/changed` signal). Returns whether it was live.
    /// Truthful allow (see `mounted_names`): hot-reload driver is test-driven
    /// until the streaming surface lands.
    #[allow(dead_code)]
    pub fn unmount(&mut self, name: &str) -> bool {
        let Some(idx) = self.mounted.iter().position(|m| m.name == name) else {
            return false;
        };
        let m = self.mounted.remove(idx);
        for (family, key) in &m.applied {
            self.slots.remove_in(family, key);
        }
        true
    }

    /// Truthful allow (same lifecycle surface as `unmount`).
    #[allow(dead_code)]
    pub fn is_mounted(&self, name: &str) -> bool {
        self.mounted.iter().any(|m| m.name == name)
    }

    /// Hot-reload / lifecycle surface (truthful allow, slots.rs precedent):
    /// driven by the unmount/reverse tests today; the runtime swap driver
    /// lands with the streaming conversation surface.
    #[allow(dead_code)]
    pub fn mounted_names(&self) -> Vec<&'static str> {
        self.mounted.iter().map(|m| m.name).collect()
    }

    /// Monotonic change counter backing the `slots/changed` event.
    #[allow(dead_code)]
    pub fn changed_revision(&self) -> u64 {
        self.slots.revision()
    }
}

/// Per-mount registration handle. Bound to one plugin's owner id and declared
/// families; every successful registration is journaled so the host can undo it.
pub struct PluginCtx<'a> {
    slots: &'a mut SlotRegistry,
    owner: &'static str,
    /// Families declared anywhere in the live composition (mounted plugins +
    /// this one). Registration outside this set is a load error.
    known: Vec<&'static str>,
    applied: Vec<(&'static str, String)>,
    owners: Vec<(&'static str, String, &'static str)>,
}

impl PluginCtx<'_> {
    /// The one composition API. Fails loud on an undeclared family or a slot
    /// key owned by another live plugin; re-registering your OWN key replaces
    /// (idempotent hot-reload).
    pub fn register<K: SlotKind>(&mut self, spec: SlotSpec) -> Result<(), PluginError> {
        let family = K::NAME;
        if !self.known.contains(&family) {
            return Err(PluginError::Undeclared {
                owner: self.owner,
                family,
            });
        }
        if let Some((_, _, prev)) = self
            .owners
            .iter()
            .find(|(f, k, _)| *f == family && *k == spec.key)
            && *prev != self.owner
        {
            return Err(PluginError::Conflict {
                owner: self.owner,
                detail: format!("slot `{family}:{}` already owned by `{prev}`", spec.key),
            });
        }
        let key = spec.key.clone();
        self.slots.register::<K>(spec);
        self.applied.push((family, key));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slots::slot_names::InputDock;

    struct Probe {
        name: &'static str,
        declares: &'static [&'static str],
        register_dock: bool,
    }
    impl Plugin for Probe {
        fn name(&self) -> &'static str {
            self.name
        }
        fn ctx_key(&self) -> &'static str {
            "ctx.probe"
        }
        fn declares(&self) -> &'static [&'static str] {
            self.declares
        }
        fn mount(&self, ctx: &mut PluginCtx) -> Result<(), PluginError> {
            if self.register_dock {
                ctx.register::<InputDock>(crate::slots::SlotSpec::new("probe", 50))?;
            }
            Ok(())
        }
    }

    #[test]
    fn boot_mounts_the_three_plugins_in_order() {
        let host = PluginHost::boot().expect("built-in composition must load");
        assert_eq!(
            host.mounted_names(),
            vec!["ui-shell", "ui-chat", "ui-control-panel"]
        );
        // The control panel's approval dock landed in the chat-declared family
        // (role-gated → visible only with the approve capability).
        let docks = crate::ui_renderer::input_docks(&host.slots, &["role:approve".to_string()]);
        assert!(
            docks
                .iter()
                .any(|d| d.key == control_panel::APPROVAL_DOCK_KEY && d.order == 5)
        );
        assert!(docks.iter().any(|d| d.key == "queue" && d.order == 20));
        // The keyed review-job renderer is the control panel's.
        assert!(
            host.slots
                .keyed::<crate::slots::slot_names::ChatNodeSlot>(control_panel::REVIEW_NODE_KIND)
                .is_some()
        );
    }

    #[test]
    fn double_declaring_a_family_fails_loud() {
        let mut host = PluginHost::new();
        host.mount(&Probe {
            name: "a",
            declares: &["settings.section"],
            register_dock: false,
        })
        .expect("first owner mounts");
        let err = host
            .mount(&Probe {
                name: "b",
                declares: &["settings.section"],
                register_dock: false,
            })
            .unwrap_err();
        assert!(
            matches!(err, PluginError::Conflict { .. }),
            "second declare of an owned family is a load error: {err}"
        );
    }

    #[test]
    fn cross_owner_slot_key_collision_fails_loud_and_rolls_back() {
        let mut host = PluginHost::boot().expect("boot");
        // A late plugin registering the approval dock key (owned by the
        // control panel) fails, and its partial registrations are undone.
        let intruder = Probe {
            name: "intruder",
            declares: &["settings.section"],
            register_dock: false,
        };
        // Register into chat-owned input.dock with a colliding key.
        struct KeyCollide;
        impl Plugin for KeyCollide {
            fn name(&self) -> &'static str {
                "collide"
            }
            fn ctx_key(&self) -> &'static str {
                "ctx.x"
            }
            fn declares(&self) -> &'static [&'static str] {
                &["tool.call.toolview"]
            }
            fn mount(&self, ctx: &mut PluginCtx) -> Result<(), PluginError> {
                ctx.register::<InputDock>(crate::slots::SlotSpec::new("queue", 99))?;
                Ok(())
            }
        }
        let err = host.mount(&KeyCollide).unwrap_err();
        let _ = intruder;
        assert!(matches!(err, PluginError::Conflict { .. }), "{err}");
        // Rollback: the queue dock still carries the chat plugin's order.
        let docks = crate::ui_renderer::input_docks(&host.slots, &["role:approve".to_string()]);
        assert_eq!(
            docks.iter().find(|d| d.key == "queue").map(|d| d.order),
            Some(20)
        );
        assert!(!host.is_mounted("collide"));
    }

    #[test]
    fn undeclared_family_registration_is_a_load_error() {
        let mut host = PluginHost::new();
        struct Rogue;
        impl Plugin for Rogue {
            fn name(&self) -> &'static str {
                "rogue"
            }
            fn ctx_key(&self) -> &'static str {
                "ctx.r"
            }
            fn declares(&self) -> &'static [&'static str] {
                &["settings.section"]
            }
            fn mount(&self, ctx: &mut PluginCtx) -> Result<(), PluginError> {
                // `root` is declared by ui-shell, which is NOT mounted here.
                use crate::slots::slot_names::Root;
                ctx.register::<Root>(crate::slots::SlotSpec::new("hijack", 0))
            }
        }
        let err = host.mount(&Rogue).unwrap_err();
        assert!(matches!(err, PluginError::Undeclared { .. }), "{err}");
    }

    #[test]
    fn unmount_reverses_registrations_and_emits_changed() {
        let mut host = PluginHost::boot().expect("boot");
        let rev = host.changed_revision();
        assert!(host.unmount("ui-control-panel"));
        assert!(!host.is_mounted("ui-control-panel"));
        let docks = crate::ui_renderer::input_docks(&host.slots, &[]);
        assert!(
            !docks
                .iter()
                .any(|d| d.key == control_panel::APPROVAL_DOCK_KEY),
            "approval dock gone after unmount"
        );
        assert!(
            host.slots
                .keyed::<crate::slots::slot_names::ChatNodeSlot>(control_panel::REVIEW_NODE_KIND)
                .is_none(),
            "review-job renderer gone after unmount"
        );
        assert!(host.changed_revision() > rev, "unmount emits slots/changed");
        // Unmounting twice is a no-op, and re-mounting works (hot reload).
        assert!(!host.unmount("ui-control-panel"));
        host.mount(&control_panel::ControlPanel)
            .expect("hot-reload remount");
        assert_eq!(host.mounted_names().len(), 3);
    }

    #[test]
    fn double_mount_same_plugin_refused() {
        let mut host = PluginHost::new();
        host.mount(&shell::Shell).expect("first");
        assert!(matches!(
            host.mount(&shell::Shell),
            Err(PluginError::DoubleMount { name: "ui-shell" })
        ));
    }

    #[test]
    fn foreign_plugin_can_compose_into_a_declared_family() {
        // The composition rule's positive side: a third plugin may insert a
        // dock between the control panel (5) and the queue (20).
        let mut host = PluginHost::boot().expect("boot");
        host.mount(&Probe {
            name: "third",
            declares: &["tool.call.toolview"],
            register_dock: true,
        })
        .expect("third-party dock");
        let docks = crate::ui_renderer::input_docks(&host.slots, &["role:approve".to_string()]);
        // Order is data: approval(5) < queue(20) < the third party's probe(50).
        let keys: Vec<&str> = docks.iter().map(|d| d.key.as_str()).collect();
        assert_eq!(keys, vec!["approval", "queue", "probe"]);
    }
}
