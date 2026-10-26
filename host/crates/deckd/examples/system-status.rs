//! Read-only live telemetry probe; does not open the deck or start a service.
fn main() {
    let mut sampler = deckd::system_status::Sampler::default();
    for _ in 0..3 {
        println!("{}", serde_json::to_string(&sampler.sample()).unwrap());
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}
