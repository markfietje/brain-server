// The desktop entry point — the webview wiring lives in the lib (shared
// with the future mobile targets); main is the thinnest possible shim.
#![forbid(unsafe_code)]

fn main() {
    brain_shell_lib::run()
}
