//! Synchronization primitives

pub use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
pub use embassy_sync::channel::Channel;
pub use embassy_sync::mutex::Mutex;
pub use embassy_sync::signal::Signal;

/// Type alias for a critical-section mutex
pub type TmMutex<T> = Mutex<CriticalSectionRawMutex, T>;

/// Type alias for a critical-section signal
pub type TmSignal<T> = Signal<CriticalSectionRawMutex, T>;

/// Type alias for a critical-section channel
pub type TmChannel<T, const N: usize> = Channel<CriticalSectionRawMutex, T, N>;
