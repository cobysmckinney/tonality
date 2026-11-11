//! Background work runs at a lower OS priority, so the UI (and the work it waits on) gets the
//! CPU first: detection, full-resolution decodes, thumbnails, sidecars and imports.

use std::sync::OnceLock;

/// How much lower: `nice` 10 (of 19). Enough that interactive threads win every time, while
/// background work still gets the whole CPU when nothing else wants it.
#[cfg(target_os = "linux")]
const NICE: i32 = 10;

/// Lower the calling thread's priority, for the rest of its life (an unprivileged thread can't
/// raise it again, so only call this on threads that only do background work). Threads it
/// starts inherit it.
pub fn lower_this_thread() {
    // On Linux, `setpriority` on a thread id affects just that thread.
    #[cfg(target_os = "linux")]
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, NICE);
    }
}

/// A low-priority pool for background work, with as many threads as cores: `install` on it and
/// parallel loops inside run here too.
pub fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .thread_name(|i| format!("background-{i}"))
            .start_handler(|_| lower_this_thread())
            .build()
            .expect("background pool")
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn pool_threads_run_at_lower_priority() {
        let nice = super::pool().install(|| unsafe {
            let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
            libc::getpriority(libc::PRIO_PROCESS, tid)
        });
        assert!(nice >= super::NICE, "nice {nice}");
    }
}
