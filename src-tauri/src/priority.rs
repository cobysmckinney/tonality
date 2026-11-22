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

/// A job handed to a pool with `spawn` panicked. The panic hook has already written it to the
/// log ([`crate::logging`]); having a handler at all is what keeps rayon from aborting the app
/// over one failed thumbnail or decode.
pub fn job_panicked(_: Box<dyn std::any::Any + Send>) {}

/// A low-priority pool for background work, with as many threads as cores: `install` on it and
/// parallel loops inside run here too.
pub fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .thread_name(|i| format!("background-{i}"))
            .start_handler(|_| lower_this_thread())
            .panic_handler(job_panicked)
            .build()
            .expect("background pool")
    })
}

/// A low-priority pool of its own for full-resolution decodes ahead of need. Not shared with
/// [`pool`]: a zoom that has to wait for a decode already under way shouldn't also wait for the
/// AI detection or thumbnails that happen to be sharing its threads.
pub fn decode_pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .thread_name(|i| format!("full-decode-{i}"))
            .start_handler(|_| lower_this_thread())
            .panic_handler(job_panicked)
            .build()
            .expect("decode pool")
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn a_panicking_job_does_not_take_the_pool_down() {
        // Without a panic handler, rayon aborts the whole process here.
        super::pool().spawn(|| panic!("job failed"));
        super::decode_pool().spawn(|| panic!("job failed"));
        assert_eq!(super::pool().install(|| 2 + 2), 4);
        assert_eq!(super::decode_pool().install(|| 2 + 2), 4);
    }

    #[test]
    fn pool_threads_run_at_lower_priority() {
        let nice = super::pool().install(|| unsafe {
            let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
            libc::getpriority(libc::PRIO_PROCESS, tid)
        });
        assert!(nice >= super::NICE, "nice {nice}");
    }
}
