#![allow(unsafe_code)]

#[cfg(unix)]
mod imp {
    use std::io;
    use std::io::Read;

    pub(crate) fn fill(buffer: &mut [u8]) -> io::Result<()> {
        let mut file = std::fs::File::open("/dev/urandom")?;
        file.read_exact(buffer)
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::io;
    use std::ptr;

    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;

    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(algorithm: *mut c_void, buffer: *mut u8, length: u32, flags: u32)
        -> i32;
    }

    pub(crate) fn fill(buffer: &mut [u8]) -> io::Result<()> {
        if buffer.is_empty() {
            return Ok(());
        }

        let length = u32::try_from(buffer.len()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "random buffer is too large")
        })?;

        let status = unsafe {
            BCryptGenRandom(
                ptr::null_mut(),
                buffer.as_mut_ptr(),
                length,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };

        if status == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(status))
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::io;

    pub(crate) fn fill(_buffer: &mut [u8]) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no secure random source is available on this platform",
        ))
    }
}

pub(crate) use imp::fill;
