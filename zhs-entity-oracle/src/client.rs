#![allow(unsafe_code)]

use std::path::{Path, PathBuf};

use zhs_entity::DataError;

#[cfg(windows)]
pub const LIBRARY_NAME: &str = "oci.dll";

#[cfg(not(windows))]
pub const LIBRARY_NAME: &str = "libclntsh.so";

pub struct Client {
    handle: platform::Handle,
    path: PathBuf,
}

impl Client {
    pub fn load() -> Result<Self, DataError> {
        let mut notes = Vec::new();

        for path in default_client_paths() {
            if !path.is_file() {
                notes.push(format!("{} (not found)", path.display()));
                continue;
            }
            match Self::load_from(&path) {
                Ok(client) => return Ok(client),
                Err(error) => notes.push(error.to_string()),
            }
        }

        match Self::load_from(Path::new(LIBRARY_NAME)) {
            Ok(client) => Ok(client),
            Err(error) => {
                notes.push(format!(
                    "{LIBRARY_NAME} from the system search path: {error}"
                ));
                Err(DataError::protocol(format!(
                    "failed to load the Oracle OCI client; {}",
                    notes.join("; ")
                )))
            }
        }
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self, DataError> {
        let path = path.as_ref();
        let handle = platform::load(path).map_err(|reason| {
            DataError::protocol(format!(
                "failed to load the Oracle OCI client {}: {reason}",
                path.display()
            ))
        })?;

        Ok(Self {
            handle,
            path: path.to_path_buf(),
        })
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn symbol(&self, name: &str) -> Result<*const (), DataError> {
        platform::symbol(self.handle, name).ok_or_else(|| {
            DataError::protocol(format!(
                "the Oracle OCI client {} does not export `{name}`",
                self.path.display()
            ))
        })
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        platform::unload(self.handle);
    }
}

unsafe impl Send for Client {}

unsafe impl Sync for Client {}

impl std::fmt::Debug for Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Client")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

pub fn default_client_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(explicit) = std::env::var_os("ZHS_OCI_LIB") {
        push_unique(&mut paths, PathBuf::from(explicit));
    }

    if let Some(home) = std::env::var_os("ORACLE_HOME") {
        let relative = if cfg!(windows) {
            Path::new("bin").join(LIBRARY_NAME)
        } else {
            Path::new("lib").join(LIBRARY_NAME)
        };
        push_unique(&mut paths, PathBuf::from(home).join(relative));
    }

    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        push_unique(&mut paths, directory.join(LIBRARY_NAME));
        push_unique(
            &mut paths,
            directory.join(".oracle-client").join(LIBRARY_NAME),
        );
    }

    push_unique(&mut paths, Path::new(".oracle-client").join(LIBRARY_NAME));

    paths
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::{CString, c_void};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    pub(super) type Handle = *mut c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *const ();
        fn FreeLibrary(module: *mut c_void) -> i32;
    }

    pub(super) fn load(path: &Path) -> Result<Handle, String> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        if handle.is_null() {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(handle)
        }
    }

    pub(super) fn symbol(handle: Handle, name: &str) -> Option<*const ()> {
        let name = CString::new(name).ok()?;

        let pointer = unsafe { GetProcAddress(handle, name.as_ptr().cast()) };
        (!pointer.is_null()).then_some(pointer)
    }

    pub(super) fn unload(handle: Handle) {
        unsafe {
            let _ = FreeLibrary(handle);
        }
    }
}

#[cfg(unix)]
mod platform {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub(super) type Handle = *mut c_void;

    const RTLD_NOW: c_int = 2;
    const RTLD_LOCAL: c_int = 0;

    #[cfg_attr(any(target_os = "linux", target_os = "android"), link(name = "dl"))]
    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
        fn dlerror() -> *mut c_char;
    }

    pub(super) fn load(path: &Path) -> Result<Handle, String> {
        let name = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| "the library path contains a NUL byte".to_owned())?;

        unsafe { dlerror() };

        let handle = unsafe { dlopen(name.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        if handle.is_null() {
            Err(last_error())
        } else {
            Ok(handle)
        }
    }

    pub(super) fn symbol(handle: Handle, name: &str) -> Option<*const ()> {
        let name = CString::new(name).ok()?;

        let pointer = unsafe { dlsym(handle, name.as_ptr()) };
        (!pointer.is_null()).then_some(pointer.cast())
    }

    pub(super) fn unload(handle: Handle) {
        unsafe {
            let _ = dlclose(handle);
        }
    }

    fn last_error() -> String {
        let message = unsafe { dlerror() };
        if message.is_null() {
            "dlopen failed without a diagnostic".to_owned()
        } else {
            unsafe { CStr::from_ptr(message) }
                .to_string_lossy()
                .into_owned()
        }
    }
}

#[cfg(not(any(windows, unix)))]
mod platform {
    use std::ffi::c_void;
    use std::path::Path;

    pub(super) type Handle = *mut c_void;

    pub(super) fn load(_path: &Path) -> Result<Handle, String> {
        Err("OCI loading is not supported on this platform".to_owned())
    }

    pub(super) fn symbol(_handle: Handle, _name: &str) -> Option<*const ()> {
        None
    }

    pub(super) fn unload(_handle: Handle) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_library_reports_the_path() {
        let error = Client::load_from("definitely-missing-zhs-oci")
            .expect_err("loading a missing library must fail");
        assert!(
            error.to_string().contains("definitely-missing-zhs-oci"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn candidate_paths_end_with_the_platform_library_name() {
        let paths = default_client_paths();
        assert!(!paths.is_empty());
        assert!(paths.iter().all(|path| path.ends_with(LIBRARY_NAME)));
    }

    #[test]
    #[ignore = "requires an OCI client (ZHS_OCI_LIB or .oracle-client)"]
    fn loads_a_real_client_and_resolves_core_symbols() {
        let client = Client::load().expect("the OCI client should be available");
        for symbol in [
            "OCIEnvCreate",
            "OCIHandleAlloc",
            "OCIErrorGet",
            "OCIStmtPrepare",
            "OCIBindByPos",
            "OCIDefineByPos",
            "OCIStmtExecute",
            "OCIStmtFetch2",
        ] {
            assert!(
                client.symbol(symbol).is_ok(),
                "the client does not export `{symbol}`"
            );
        }
    }
}
