//! The tokio bridge (§4): the runtime it builds, and the rule it exists to
//! keep — **nothing may ever call `block_on` on the UI thread**.
//!
//! That rule is kept by construction: `TokioBridge`'s `Handle` is a private
//! field with no accessor, so the only shape the module hands out is a spawn,
//! and a caller who cannot name a `Handle` cannot block on one. Privacy is
//! the compiler's job, not a test's — what a test *can* do is fail the day
//! someone grows a way out, and
//! `the_bridge_hands_out_no_runtime_handle_and_blocks_on_nothing_but_its_own_thread`
//! is that guard: it reads the crate's own source and checks the shape.
use deck_gui_core::tokio_bridge::{THREAD_NAME, WORKER_THREADS};
use deck_gui_core::TokioBridge;
use std::sync::mpsc;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(5);

/// Every module of the crate, so the guard below cannot be dodged by moving
/// the offending line to a neighbouring file.
const SOURCES: [(&str, &str); 5] = [
    ("lib.rs", include_str!("../src/lib.rs")),
    ("events.rs", include_str!("../src/events.rs")),
    ("tokio_bridge.rs", include_str!("../src/tokio_bridge.rs")),
    ("api_client.rs", include_str!("../src/api_client.rs")),
    ("host_state.rs", include_str!("../src/host_state.rs")),
];

#[test]
fn the_bridge_hands_out_no_runtime_handle_and_blocks_on_nothing_but_its_own_thread() {
    let bridge = SOURCES
        .iter()
        .find(|(name, _)| *name == "tokio_bridge.rs")
        .expect("the bridge's source")
        .1;

    // 1. No blocking call anywhere but the bridge's own std thread.
    //
    //    The vocabulary is wider than `block_on(` on purpose: a review walked
    //    straight past the narrow version with `oneshot::Receiver::blocking_recv`,
    //    which parks the caller just as thoroughly and does not contain the
    //    word. `Reply` is now a newtype that implements `Future` and nothing
    //    else, so that particular door is shut by the compiler rather than by
    //    this list — but the list is what catches the next one.
    const PARKS: [&str; 7] = [
        "block_on(",
        "blocking_recv(",
        "recv_blocking(",
        "blocking_send(",
        "blocking_lock(",
        "Handle::current(",
        "thread::park(",
    ];
    for (name, source) in SOURCES {
        // Comments describe these calls; only real code may not make them.
        let code: String = source
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for park in PARKS {
            let calls = code.matches(park).count();
            let expected = usize::from(park == "block_on(" && name == "tokio_bridge.rs");
            assert_eq!(
                calls, expected,
                "{name} calls {park} {calls} time(s); only the bridge's own thread may park"
            );
        }
    }
    assert!(
        bridge.contains("runtime.block_on(async {"),
        "the one block_on is the shutdown future on the bridge's thread"
    );

    // 2. No way to get the `Handle` out: no accessor, no public field, no
    //    return type. `spawn` is the only door, and it takes a future.
    assert!(
        bridge.contains("    handle: Handle,"),
        "the handle is a private field"
    );
    for escape in [
        "pub fn handle",
        "pub handle",
        "-> Handle",
        "-> &Handle",
        "-> tokio::runtime::Handle",
        "Deref<Target = Handle>",
        "impl AsRef<Handle>",
    ] {
        assert!(
            !bridge.contains(escape),
            "the bridge exposes its runtime handle through {escape:?}"
        );
    }
    assert!(
        bridge.contains("pub fn spawn<F>(&self, task: F) -> tokio::task::JoinHandle<F::Output>"),
        "spawn is the spawn-shaped door, and it hands back a task handle"
    );

    // 3. Nor does `ApiClient` hand one on: `call` is synchronous and gives
    //    back a `oneshot::Receiver` for the UI thread to await, never a
    //    runtime to block on.
    let client = SOURCES
        .iter()
        .find(|(name, _)| *name == "api_client.rs")
        .expect("the client's source")
        .1;
    assert!(
        client.contains("pub fn call(&self, method: &str, params: Value) -> Reply {"),
        "call hands back a Reply, not a receiver"
    );
    // `Reply` is the point: a `oneshot::Receiver` carries `blocking_recv()`,
    // which parks whichever thread calls it, and the thread that draws the
    // window may never park. A newtype that implements `Future` and exposes
    // nothing else makes that the compiler's rule instead of this file's.
    assert!(
        client.contains("pub struct Reply(oneshot::Receiver<Result<Value, ClientError>>);"),
        "the receiver is wrapped, so its blocking half is unreachable"
    );
    assert!(
        client.contains("impl std::future::Future for Reply"),
        "and the wrapper is awaitable, or a view could not use it at all"
    );
    for leak in [
        "pub fn into_inner",
        "pub fn receiver",
        "-> oneshot::Receiver",
    ] {
        assert!(
            !client.contains(leak),
            "Reply hands its receiver back out through {leak:?}"
        );
    }
    for escape in ["Handle::current", "runtime.enter", "-> Handle"] {
        assert!(
            !client.contains(escape),
            "the client leaks a runtime through {escape:?}"
        );
    }
}

#[test]
fn the_bridge_runs_two_workers_on_its_own_threads() {
    let bridge = TokioBridge::start().expect("a runtime thread");
    assert_eq!(bridge.workers(), WORKER_THREADS, "§8.1: worker_threads(2)");
    let here = std::thread::current().id();
    let (tx, rx) = mpsc::channel();
    bridge.spawn(async move {
        let workers = tokio::runtime::Handle::current().metrics().num_workers();
        let name = std::thread::current()
            .name()
            .unwrap_or_default()
            .to_string();
        let _ = tx.send((workers, std::thread::current().id(), name));
    });
    let (workers, there, name) = rx.recv_timeout(WAIT).expect("the task ran within 5 s");
    assert_eq!(workers, WORKER_THREADS);
    assert_ne!(there, here, "a spawned task never runs on the caller");
    assert!(name.starts_with(THREAD_NAME), "{name:?}");
}

#[test]
fn a_spawned_task_answers_the_caller_without_the_caller_blocking_on_the_runtime() {
    // What a view does: hand the work to the bridge, get a receiver back,
    // and await it on its own executor. Here the "own executor" is a
    // current-thread runtime standing in for gpui's foreground one — the
    // point is that it is not the bridge's, and the bridge was never asked
    // to block anything.
    let bridge = TokioBridge::start().expect("a runtime thread");
    let (tx, rx) = tokio::sync::oneshot::channel();
    bridge.spawn(async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        let _ = tx.send(41 + 1);
    });
    let ui = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let answer = ui.block_on(async { tokio::time::timeout(WAIT, rx).await });
    assert_eq!(answer.unwrap().unwrap(), 42);
}

#[test]
fn a_dropped_bridge_ends_its_runtime_thread() {
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    {
        let bridge = TokioBridge::start().expect("a runtime thread");
        let flag = running.clone();
        bridge.spawn(async move {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = tx.send(());
            // Never finishes: the drop has to cancel it, not wait for it.
            std::future::pending::<()>().await;
        });
        rx.recv_timeout(WAIT).expect("the task started");
        assert!(running.load(std::sync::atomic::Ordering::SeqCst));
        // `shutdown` joins the thread; if the pending task could wedge it,
        // this test would hang rather than fail, which is the honest signal.
        bridge.shutdown();
    }
    // A second bridge in the same process is fine: nothing here is global.
    let again = TokioBridge::start().expect("a second runtime thread");
    assert_eq!(again.workers(), WORKER_THREADS);
}
