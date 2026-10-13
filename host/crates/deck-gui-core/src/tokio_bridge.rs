//! The tokio bridge (§8.1, spelled out there because it is the one piece
//! openmicrokbd has no precedent for: its IPC is std threads and `post`, and
//! Zed's `gpui_tokio` is a Zed-workspace crate against Zed-main gpui, not on
//! crates.io and not buildable against `gpui =0.2.2`).
//!
//! One std thread owns a two-worker multi-thread runtime and does nothing but
//! block on a shutdown future. Everything else reaches that runtime through
//! [`TokioBridge::spawn`].
//!
//! **Nothing may ever call `block_on` on the UI thread.** That is kept here by
//! construction and not by discipline: the runtime `Handle` is a private
//! field with no accessor, so the only shape this module hands out is a spawn.
//! A UI thread that cannot name a `Handle` cannot block on one.
//! `tests/bridge.rs` is the guard that keeps it that way.
use std::future::Future;
use std::io;
use std::thread::JoinHandle;
use tokio::runtime::Handle;
use tokio::sync::oneshot;

/// §8.1's `worker_threads(2)`: the GUI's I/O is one socket and a handful of
/// calls, and two workers keep a slow `state.get` from parking the reader.
pub const WORKER_THREADS: usize = 2;

/// The runtime thread's name, so a sample or a crash log says which thread
/// this is.
pub const THREAD_NAME: &str = "deckd-gui-tokio";

pub struct TokioBridge {
    /// Private, and deliberately without an accessor: see the module note.
    handle: Handle,
    /// Dropped or sent to end the runtime thread's `block_on`.
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for TokioBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokioBridge")
            .field("workers", &self.handle.metrics().num_workers())
            .field("running", &self.thread.is_some())
            .finish()
    }
}

impl TokioBridge {
    /// Start the runtime thread and wait for it to be ready. `Err` only when
    /// the thread or the runtime could not be created at all, which on this
    /// platform means the process is out of threads.
    pub fn start() -> io::Result<TokioBridge> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<io::Result<Handle>>();
        let (shutdown, stop) = oneshot::channel();
        let thread = std::thread::Builder::new()
            .name(THREAD_NAME.to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(WORKER_THREADS)
                    .enable_all()
                    .thread_name(THREAD_NAME)
                    .build();
                let runtime = match runtime {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                if ready_tx.send(Ok(runtime.handle().clone())).is_err() {
                    return;
                }
                // The crate's one and only `block_on`, and it is on *this*
                // thread: a std thread that exists to hold the runtime open.
                // The UI thread never has a `Handle` to do the same with.
                runtime.block_on(async {
                    let _ = stop.await;
                });
            })?;
        let handle = ready_rx
            .recv()
            .map_err(|_| io::Error::other("the tokio thread ended before it was ready"))??;
        Ok(TokioBridge {
            handle,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    /// Run `task` on the bridge. The only way in, and the only thing the UI
    /// thread ever asks of tokio: a view awaits the `oneshot::Receiver` an
    /// [`crate::ApiClient::call`] handed it on gpui's own executor, and the
    /// task spawned here does the socket I/O.
    pub fn spawn<F>(&self, task: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.handle.spawn(task)
    }

    /// How many workers the runtime was built with. For the Service screen
    /// and for the test that pins §8.1's `worker_threads(2)`.
    pub fn workers(&self) -> usize {
        self.handle.metrics().num_workers()
    }

    /// End the runtime and join its thread. Spawned tasks are cancelled at
    /// their next await point. `Drop` does the same, so this is only for a
    /// quit that wants to be sure the thread is gone before it returns.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for TokioBridge {
    fn drop(&mut self) {
        self.stop();
    }
}
