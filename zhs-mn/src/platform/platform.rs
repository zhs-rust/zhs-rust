#![allow(unsafe_code)]

#[cfg(windows)]
mod imp {

    pub(crate) const DEFAULT_GRANULARITY_MILLIS: f64 = 15.625;

    #[link(name = "winmm")]
    unsafe extern "system" {
        fn timeBeginPeriod(period: u32) -> u32;
        fn timeEndPeriod(period: u32) -> u32;
    }

    pub(crate) fn begin(millis: u32) -> bool {
        unsafe { timeBeginPeriod(millis) == 0 }
    }

    pub(crate) fn end(millis: u32) {
        unsafe {
            let _ = timeEndPeriod(millis);
        }
    }
}

#[cfg(not(windows))]
mod imp {

    pub(crate) const DEFAULT_GRANULARITY_MILLIS: f64 = 1.0;

    pub(crate) fn begin(_millis: u32) -> bool {
        false
    }

    pub(crate) fn end(_millis: u32) {}
}

pub(crate) use imp::{DEFAULT_GRANULARITY_MILLIS, begin, end};
