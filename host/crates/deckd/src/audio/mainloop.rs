//! **B10** — the main thread, given to a CFRunLoop, and the way everything
//! else gets work onto it.
//!
//! §2.3 puts a CFRunLoop on `main` and nothing implemented it: `main` owned
//! the tokio runtime and the signal handlers, and the run loop existed only
//! in the diagram. That was survivable while nothing spoke, and V13 makes it
//! fatal:
//!
//! > AVSpeech buffer callbacks are delivered on the **main thread only**.
//! > Measured: a worker thread with no run loop → **0 buffers**; a worker
//! > driving its own run loop → **0 buffers**; the main thread driving its
//! > run loop → **300/300 buffers**.
//!
//! The same applies to `SFSpeechRecognizer`'s result handlers and to the TCC
//! prompts of §7.8. So a daemon with no run loop on main is a daemon that
//! cannot speak and cannot listen, however correct everything else is.
//!
//! The hop is `dispatch_async_f` on the main queue — **not** `dispatch_async`
//! with a block. A plain `extern "C" fn` context pointer needs no `block2`,
//! no `dispatch2` and no extra feature on any pinned crate, and the thing
//! being posted is a `Box<dyn FnOnce>` either way.
//!
//! What comes back is a `crossbeam` oneshot, because every caller of this is
//! already in a `crossbeam` world (`deckd-audio`, `deckd-tts`) and a tokio
//! oneshot would drag a runtime handle onto threads that have none.
use crossbeam_channel::{bounded, Receiver};
use objc2_core_foundation::CFRunLoop;
use std::ffi::c_void;

extern "C" {
    /// `dispatch_get_main_queue()` is a macro over this symbol, so there is
    /// no function to call: the queue *is* the address of this global.
    static _dispatch_main_q: [u8; 0];

    /// The block-free form. `work(context)` runs on `queue`, once.
    fn dispatch_async_f(
        queue: *mut c_void,
        context: *mut c_void,
        work: extern "C-unwind" fn(*mut c_void),
    );
}

/// One posted job.
type Job = Box<dyn FnOnce() + Send + 'static>;

extern "C-unwind" fn trampoline(context: *mut c_void) {
    if context.is_null() {
        return;
    }
    // SAFETY: `context` is the `Box<Job>` that `post` leaked, and
    // `dispatch_async_f` calls this exactly once per post.
    let job: Box<Job> = unsafe { Box::from_raw(context.cast::<Job>()) };
    // A panicking job must not unwind into libdispatch: that is an abort
    // with no message and no thread name, which is the worst possible way
    // to learn that a voice was missing from the pool.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
}

/// Run `f` on the main thread. Returns at once; nothing is awaited.
///
/// The job runs when the main run loop next turns, which is why
/// [`run_forever`] has to be called or this is a queue that never drains.
pub fn post(f: impl FnOnce() + Send + 'static) {
    let job: Job = Box::new(f);
    let context = Box::into_raw(Box::new(job)).cast::<c_void>();
    // SAFETY: `_dispatch_main_q`'s address is the main queue, `trampoline`
    // has the required signature, and it takes ownership of `context`.
    unsafe {
        dispatch_async_f(
            std::ptr::addr_of!(_dispatch_main_q) as *mut c_void,
            context,
            trampoline,
        );
    }
}

/// Run `f` on the main thread and hand the answer back (§2.3's
/// "`dispatch_async(main_queue)` + a oneshot back").
///
/// The receiver is **bounded(1)** rather than a rendezvous: the poster must
/// be free to give up on the answer — a `Listen` that was aborted while the
/// main thread was blocked in a TCC prompt is exactly that case — and
/// dropping the receiver must not block the main thread on the send.
pub fn call<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = bounded(1);
    post(move || {
        let _ = tx.send(f());
    });
    rx
}

/// Give this thread to a CFRunLoop, for ever (B10).
///
/// Called on `main`, after the tokio runtime and the core are up: they own
/// their own threads, and what is left for `main` is precisely the ObjC work
/// that can run nowhere else.
pub fn run_forever() {
    CFRunLoop::run();
}

/// Ask the main run loop to return from [`run_forever`].
///
/// Used at shutdown. It is safe from any thread: `CFRunLoopStop` is one of
/// the few CoreFoundation calls that is.
pub fn stop() {
    if let Some(main) = CFRunLoop::main() {
        main.stop();
    }
}

/// Turn the main run loop for at most `seconds`, returning after the first
/// source it handles.
///
/// This is how a **test** drains what [`post`] queued without giving the
/// thread away for ever. It is not part of the daemon's path: the daemon
/// calls [`run_forever`].
pub fn turn(seconds: f64) -> bool {
    use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoopRunResult};
    // SAFETY: reading the framework's own mode constant.
    let mode = unsafe { kCFRunLoopDefaultMode };
    CFRunLoop::run_in_mode(mode, seconds, true) == CFRunLoopRunResult::HandledSource
}

/// Turn the main run loop for `budget`, draining everything posted to it.
///
/// This is what `deckd run` calls on `main` instead of `run_forever`, once
/// per supervision period. The difference matters: `run_forever` never
/// returns, so the daemon could not also watch its stop flag and its core
/// thread from `main` — and `CFRunLoopStop` from a signal handler is not
/// async-signal-safe. Posted ObjC work runs here exactly as it would under
/// `CFRunLoopRun`; §2.3's "main thread CFRunLoop" is satisfied by a loop
/// that is always either inside the run loop or about to be.
///
/// The `sleep` is not padding. `CFRunLoopRunInMode` returns
/// `kCFRunLoopRunFinished` **immediately** when the mode has no sources at
/// all, which is the daemon's state until something posts, so a bare
/// `while { run_in_mode }` would burn a core for the process's whole life.
pub fn pump(budget: std::time::Duration) {
    use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoopRunResult};
    // SAFETY: reading the framework's own mode constant.
    let mode = unsafe { kCFRunLoopDefaultMode };
    let deadline = std::time::Instant::now() + budget;
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return;
        }
        let left = deadline - now;
        if CFRunLoop::run_in_mode(mode, left.as_secs_f64(), true)
            != CFRunLoopRunResult::HandledSource
        {
            std::thread::sleep(left.min(std::time::Duration::from_millis(10)));
        }
    }
}

/// Is this the process's main thread? `dispatch_async_f` on the main queue
/// runs **only** there (that is what makes it the main queue), so a caller
/// that wants to see posted work has to be it.
#[must_use]
pub fn is_main_thread() -> bool {
    // SAFETY: `pthread_main_np` takes nothing and returns an int.
    extern "C" {
        fn pthread_main_np() -> i32;
    }
    unsafe { pthread_main_np() != 0 }
}
