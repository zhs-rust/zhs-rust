use std::io;

pub(crate) fn fill(buffer: &mut [u8]) -> io::Result<()> {
    imp::fill(buffer)
}

#[cfg(unix)]
mod imp {
    use std::fs::File;
    use std::io::{self, Read};

    pub(super) fn fill(buffer: &mut [u8]) -> io::Result<()> {
        let mut file = File::open("/dev/urandom")?;
        file.read_exact(buffer)
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::io;

    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;

    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(algorithm: *mut c_void, buffer: *mut u8, len: u32, flags: u32) -> i32;
    }

    pub(super) fn fill(buffer: &mut [u8]) -> io::Result<()> {
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };

        if status == 0 {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "BCryptGenRandom failed with NTSTATUS {status:#x}"
            )))
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::io;

    pub(super) fn fill(_buffer: &mut [u8]) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no secure random source is available on this platform",
        ))
    }
}
