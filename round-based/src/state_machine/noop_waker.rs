const VTABLE: core::task::RawWakerVTable = core::task::RawWakerVTable::new(
    // Cloning just returns a new no-op raw waker
    |_| NOOP_RAW_WAKER,
    // `wake` does nothing
    |_| {},
    // `wake_by_ref` does nothing
    |_| {},
    // Dropping does nothing as we don't allocate anything
    |_| {},
);

const NOOP_RAW_WAKER: core::task::RawWaker = core::task::RawWaker::new(core::ptr::null(), &VTABLE);

/// Returns no-op waker which does nothing when wake is called
///
/// It should be removed once [rust#98286](https://github.com/rust-lang/rust/issues/98286) is
/// stabilized
pub fn noop_waker() -> core::task::Waker {
    unsafe { core::task::Waker::from_raw(NOOP_RAW_WAKER) }
}
