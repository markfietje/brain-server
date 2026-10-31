//! W-T1 — the pkg's thinnest consumer of the detection core. ONE
//! implementation, two surfaces: the wizard consumes `detect_cards` over the
//! IPC seam; this binary prints the SAME report so the Tools pkg's
//! postinstall can honor the D1 preserve law without re-implementing it in
//! shell. Read-only by the core's own law (the ZERO-WRITE source-scan pins
//! the module, and this binary adds no write of its own).
//!
//! Output: the report as one JSON line (machine-readable). `--fields` prints
//! the postinstall's parse surface instead — flat `key=value` lines for the
//! D1 facts, because a shell script parsing JSON is a second parser waiting
//! to drift. Exit 0 with a report; exit 2 when the surface does not exist
//! (the store build compiles the detection core out — this binary refuses,
//! it does not guess).

fn main() {
    let fields = std::env::args().any(|arg| arg == "--fields");

    #[cfg(not(feature = "mas"))]
    {
        let home = match std::env::var("HOME") {
            Ok(home) => home,
            Err(_) => {
                eprintln!("stewardmem-detect: HOME is not set — refusing to guess the paths");
                std::process::exit(2);
            }
        };
        let report = brain_shell_lib::detect::detect_all(std::path::Path::new(&home), None);
        if fields {
            if let Some(existing) = &report.existing {
                println!("existing.binaryPath={}", existing.binary_path);
                if let Some(version) = &existing.version {
                    println!("existing.version={version}");
                }
                println!("existing.running={}", existing.running);
            }
            if let Some(openclaw) = &report.openclaw {
                println!("openclaw.pluginEnabled={}", openclaw.plugin_enabled);
            }
            println!("kernel.running={}", report.kernel_on_port.running);
            // Every other fact stays in the JSON mode; the postinstall's
            // decision needs exactly the D1 facts above.
            return;
        }
        println!(
            "{}",
            serde_json::to_string(&report).expect("the report serializes")
        );
    }

    #[cfg(feature = "mas")]
    {
        let _ = fields;
        eprintln!("stewardmem-detect: the detection core is direct-channel only");
        std::process::exit(2);
    }
}
