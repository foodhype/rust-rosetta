// Implements http://rosettacode.org/wiki/Metered_concurrency
// Rust has a perfectly good Semaphore type already.  It lacks count(), though, so we can't use it
// directly.

#![feature(unsafe_destructor)]
#![feature(old_io)]
#![feature(std_misc)]
#![feature(core)]

use std::old_io::timer;
use std::sync::Arc;
use std::sync::atomic::{AtomicUint, Ordering};
use std::time::duration::Duration;
use std::thread::spawn;
use std::sync::mpsc::channel;

pub struct CountingSemaphore {
    count: AtomicUint, // Remaining resource count
    backoff: Duration, // How long to sleep if a resource is being contended
}

pub struct CountingSemaphoreGuard<'a> {
    sem: &'a CountingSemaphore, // A reference to the owning semaphore.
}

impl CountingSemaphore {
    // Create a semaphore with `max` available resources and a linearly increasing backoff of
    // `backoff` (used during spinlock contention).
    pub fn new(max: usize, backoff: Duration) -> CountingSemaphore {
        CountingSemaphore { count: AtomicUint::new(max), backoff: backoff }
    }

    // Acquire a resource, returning a RAII CountingSemaphoreGuard.
    pub fn acquire(&self) -> CountingSemaphoreGuard {
        // Spinlock until remaining resource count is at least 1
        let mut backoff: Duration = self.backoff;
        loop {
            // Probably don't need SeqCst here, but it doesn't hurt.
            let count = self.count.load(Ordering::SeqCst);
            // The check for 0 is necessary to make sure we don't go negative, which is why this
            // must be a compare-and-swap rather than a straight decrement.
            if count == 0 || self.count.compare_and_swap(count, count - 1, Ordering::SeqCst) != count {
                // Linear backoff a la Servo's spinlock contention.
                timer::sleep(backoff);
                backoff = backoff + self.backoff;
            } else {
                // We successfully acquired the resource.
                break
            }
        }
        CountingSemaphoreGuard { sem: self }
    }

    // Return remaining resource count
    pub fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

#[unsafe_destructor]
impl<'a> Drop for CountingSemaphoreGuard<'a> {
    // When the guard is dropped, a resource is released back to the pool.
    fn drop(&mut self) {
        self.sem.count.fetch_add(1, Ordering::SeqCst);
    }
}

fn metered(duration: Duration) {
    static MAX_COUNT: usize = 4; // Total available resources
    static NUM_WORKERS: u8 = 10; // Number of workers contending for the resources
    let backoff = Duration::milliseconds(1); // Linear backoff time
    // Create a shared reference to the semaphore
    let sem = Arc::new(CountingSemaphore::new(MAX_COUNT, backoff));
    // Create a channel for notifying the main task that the workers are done
    let (tx, rx) = channel();
    for i in (0..NUM_WORKERS) {
        let sem = sem.clone();
        let tx = tx.clone();
        spawn(move || -> () {
            // Acquire the resource
            let guard = sem.acquire();
            let count = sem.count();
            // Make sure the count is legal
            assert!(count < MAX_COUNT);
            println!("Worker {} after acquire: count = {}", i, count);
            // Sleep for `duration`
            timer::sleep(duration);
            // Release the resource
            drop(guard);
            // Make sure the count is legal
            let count = sem.count();
            assert!(count <= MAX_COUNT);
            println!("Worker {} after release: count = {}", i, count);
            // Notify the main task of completion
            tx.send(()).unwrap();
        });
    }
    drop(tx);
    // Wait for all the subtasks to finish
    for _ in (0..NUM_WORKERS) {
        rx.recv().unwrap();
    }
}

#[test]
fn test_metered_concurrency() {
    // Hold each resource for 1/20 of a second per worker
    metered(Duration::seconds(1) / 20);
}

#[cfg(not(test))]
fn main() {
    // Hold each resource for 2 seconds per worker
    metered(Duration::seconds(2));
}
