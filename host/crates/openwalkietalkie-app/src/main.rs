//! `OpenWalkieTalkie.app`'s entry point. Everything is in the library beside
//! it, so the crate's own tests can reach it without opening a window.
fn main() {
    if let Err(e) = openwalkietalkie_app::run() {
        // There is no window to say this in: the tokio bridge is what a
        // window would have talked to the daemon through.
        eprintln!("openwalkietalkie-app: {e}");
        std::process::exit(1);
    }
}
