#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;

use zhs_entity::DataError;

use crate::client::Client;

pub(crate) type Handle = *mut c_void;

pub(crate) const OCI_THREADED: u32 = 0x0000_0001;

pub(crate) const OCI_AL32UTF8: u16 = 873;

pub(crate) const OCI_SUCCESS: i32 = 0;
pub(crate) const OCI_SUCCESS_WITH_INFO: i32 = 1;
pub(crate) const OCI_NO_DATA: i32 = 100;

pub(crate) const OCI_HTYPE_ENV: u32 = 1;
pub(crate) const OCI_HTYPE_ERROR: u32 = 2;
pub(crate) const OCI_HTYPE_STMT: u32 = 4;
pub(crate) const OCI_DTYPE_PARAM: u32 = 53;
pub(crate) const OCI_DTYPE_LOB: u32 = 50;
pub(crate) const OCI_ONE_PIECE: u8 = 0;
pub(crate) const SQLCS_IMPLICIT: u8 = 1;

pub(crate) const OCI_ATTR_DATA_SIZE: u32 = 1;
pub(crate) const OCI_ATTR_DATA_TYPE: u32 = 2;
pub(crate) const OCI_ATTR_NAME: u32 = 4;
pub(crate) const OCI_ATTR_ROW_COUNT: u32 = 9;
pub(crate) const OCI_ATTR_PARAM_COUNT: u32 = 18;
pub(crate) const OCI_ATTR_STMT_TYPE: u32 = 24;

pub(crate) const OCI_STMT_SELECT: u16 = 1;
pub(crate) const OCI_FETCH_NEXT: u16 = 2;
pub(crate) const OCI_NTV_SYNTAX: u32 = 1;
pub(crate) const OCI_DEFAULT: u32 = 0;

pub(crate) const SQLT_CHR: u16 = 1;
pub(crate) const SQLT_NUM: u16 = 2;
pub(crate) const SQLT_INT: u16 = 3;
pub(crate) const SQLT_FLT: u16 = 4;
pub(crate) const SQLT_VNU: u16 = 6;
pub(crate) const SQLT_BDOUBLE: u16 = 22;
pub(crate) const SQLT_BIN: u16 = 23;
pub(crate) const SQLT_IBFLOAT: u16 = 100;
pub(crate) const SQLT_IBDOUBLE: u16 = 101;
pub(crate) const SQLT_CLOB: u16 = 112;
pub(crate) const SQLT_BLOB: u16 = 113;

pub(crate) type Sword = i32;

type EnvNlsCreate = unsafe extern "C" fn(
    *mut *mut c_void,
    u32,
    *const c_void,
    *const c_void,
    *const c_void,
    *const c_void,
    usize,
    *mut *mut c_void,
    u16,
    u16,
) -> Sword;
type EnvCreate = unsafe extern "C" fn(
    *mut *mut c_void,
    u32,
    *const c_void,
    *const c_void,
    *const c_void,
    *const c_void,
    usize,
    *mut *mut c_void,
) -> Sword;
type HandleAlloc =
    unsafe extern "C" fn(*const c_void, *mut *mut c_void, u32, usize, *mut *mut c_void) -> Sword;
type HandleFree = unsafe extern "C" fn(*mut c_void, u32) -> Sword;
type ErrorGet =
    unsafe extern "C" fn(*mut c_void, u32, *mut u8, *mut i16, *mut u8, u32, u32) -> Sword;
type Logon2 = unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut *mut c_void,
    *const u8,
    u32,
    *const u8,
    u32,
    *const u8,
    u32,
    u32,
) -> Sword;
type Logoff = unsafe extern "C" fn(*mut c_void, *mut c_void) -> Sword;
type StmtPrepare =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *const u8, u32, u32, u32) -> Sword;
type StmtExecute = unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    u32,
    u32,
    *const c_void,
    *mut c_void,
    u32,
) -> Sword;
type StmtFetch2 = unsafe extern "C" fn(*mut c_void, *mut c_void, u32, u16, u32, u32) -> Sword;
type BindByPos = unsafe extern "C" fn(
    *mut c_void,
    *mut *mut c_void,
    *mut c_void,
    u32,
    *mut c_void,
    i32,
    u16,
    *mut c_void,
    *mut u16,
    *mut u16,
    u32,
    *mut u32,
    u32,
) -> Sword;
type DefineByPos = unsafe extern "C" fn(
    *mut c_void,
    *mut *mut c_void,
    *mut c_void,
    u32,
    *mut c_void,
    i32,
    u16,
    *mut c_void,
    *mut u16,
    *mut u16,
    u32,
) -> Sword;
type AttrGet =
    unsafe extern "C" fn(*const c_void, u32, *mut c_void, *mut u32, u32, *mut c_void) -> Sword;
type ParamGet =
    unsafe extern "C" fn(*const c_void, u32, *mut c_void, *mut *mut c_void, u32) -> Sword;
type Trans = unsafe extern "C" fn(*mut c_void, *mut c_void, u32) -> Sword;
type DescriptorAlloc =
    unsafe extern "C" fn(*const c_void, *mut *mut c_void, u32, usize, *mut *mut c_void) -> Sword;
type DescriptorFree = unsafe extern "C" fn(*mut c_void, u32) -> Sword;
type LobGetLength = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut u32) -> Sword;
type LobRead2 = unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut u64,
    *mut u64,
    u64,
    *mut c_void,
    u64,
    u8,
    *mut c_void,
    *const c_void,
    u16,
    u8,
) -> Sword;

pub(crate) struct Functions {
    pub(crate) env_nls_create: EnvNlsCreate,
    pub(crate) env_create: EnvCreate,
    pub(crate) handle_alloc: HandleAlloc,
    pub(crate) handle_free: HandleFree,
    pub(crate) error_get: ErrorGet,
    pub(crate) logon2: Logon2,
    pub(crate) logoff: Logoff,
    pub(crate) stmt_prepare: StmtPrepare,
    pub(crate) stmt_execute: StmtExecute,
    pub(crate) stmt_fetch2: StmtFetch2,
    pub(crate) bind_by_pos: BindByPos,
    pub(crate) define_by_pos: DefineByPos,
    pub(crate) attr_get: AttrGet,
    pub(crate) param_get: ParamGet,
    pub(crate) trans_commit: Trans,
    pub(crate) trans_rollback: Trans,
    pub(crate) descriptor_alloc: DescriptorAlloc,
    pub(crate) descriptor_free: DescriptorFree,
    pub(crate) lob_get_length: LobGetLength,
    pub(crate) lob_read2: LobRead2,
}

pub(crate) struct Oci {
    _client: Client,
    functions: Functions,
}

impl Oci {
    pub(crate) fn load() -> Result<Self, DataError> {
        let client = Client::load()?;
        let functions = Functions::load(&client)?;
        Ok(Self {
            _client: client,
            functions,
        })
    }

    pub(crate) fn functions(&self) -> &Functions {
        &self.functions
    }
}

impl Functions {
    fn load(client: &Client) -> Result<Self, DataError> {
        macro_rules! resolve {
            ($name:literal, $ty:ty) => {{
                let pointer = client.symbol($name)?;

                unsafe { std::mem::transmute::<*const (), $ty>(pointer) }
            }};
        }

        Ok(Self {
            env_nls_create: resolve!("OCIEnvNlsCreate", EnvNlsCreate),
            env_create: resolve!("OCIEnvCreate", EnvCreate),
            handle_alloc: resolve!("OCIHandleAlloc", HandleAlloc),
            handle_free: resolve!("OCIHandleFree", HandleFree),
            error_get: resolve!("OCIErrorGet", ErrorGet),
            logon2: resolve!("OCILogon2", Logon2),
            logoff: resolve!("OCILogoff", Logoff),
            stmt_prepare: resolve!("OCIStmtPrepare", StmtPrepare),
            stmt_execute: resolve!("OCIStmtExecute", StmtExecute),
            stmt_fetch2: resolve!("OCIStmtFetch2", StmtFetch2),
            bind_by_pos: resolve!("OCIBindByPos", BindByPos),
            define_by_pos: resolve!("OCIDefineByPos", DefineByPos),
            attr_get: resolve!("OCIAttrGet", AttrGet),
            param_get: resolve!("OCIParamGet", ParamGet),
            trans_commit: resolve!("OCITransCommit", Trans),
            trans_rollback: resolve!("OCITransRollback", Trans),
            descriptor_alloc: resolve!("OCIDescriptorAlloc", DescriptorAlloc),
            descriptor_free: resolve!("OCIDescriptorFree", DescriptorFree),
            lob_get_length: resolve!("OCILobGetLength", LobGetLength),
            lob_read2: resolve!("OCILobRead2", LobRead2),
        })
    }
}

pub(crate) struct Environment {
    oci: Arc<Oci>,
    handle: Handle,
}

unsafe impl Send for Environment {}

unsafe impl Sync for Environment {}

impl Environment {
    pub(crate) fn create(oci: Arc<Oci>) -> Result<Self, DataError> {
        let functions = oci.functions();
        let mut handle: Handle = ptr::null_mut();

        let status = unsafe {
            (functions.env_nls_create)(
                &mut handle,
                OCI_THREADED,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                0,
                ptr::null_mut(),
                OCI_AL32UTF8,
                OCI_AL32UTF8,
            )
        };

        if status != OCI_SUCCESS {
            let fallback = unsafe {
                (functions.env_create)(
                    &mut handle,
                    OCI_THREADED,
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    0,
                    ptr::null_mut(),
                )
            };
            if fallback != OCI_SUCCESS {
                return Err(DataError::protocol(format!(
                    "failed to create the OCI environment (status {fallback})"
                )));
            }
        }

        Ok(Self { oci, handle })
    }

    pub(crate) fn handle(&self) -> Handle {
        self.handle
    }

    pub(crate) fn oci(&self) -> &Arc<Oci> {
        &self.oci
    }

    pub(crate) fn allocate(&self, handle_type: u32) -> Result<Handle, DataError> {
        let functions = self.oci.functions();
        let mut handle: Handle = ptr::null_mut();

        let status = unsafe {
            (functions.handle_alloc)(self.handle, &mut handle, handle_type, 0, ptr::null_mut())
        };
        if status != OCI_SUCCESS {
            return Err(DataError::protocol(format!(
                "failed to allocate an OCI handle of type {handle_type} (status {status})"
            )));
        }
        Ok(handle)
    }

    pub(crate) fn free(&self, handle: Handle, handle_type: u32) {
        if handle.is_null() {
            return;
        }

        unsafe {
            let _ = (self.oci.functions().handle_free)(handle, handle_type);
        }
    }

    pub(crate) fn descriptor(&self, descriptor_type: u32) -> Result<Handle, DataError> {
        let mut handle: Handle = ptr::null_mut();

        let status = unsafe {
            (self.oci.functions().descriptor_alloc)(
                self.handle,
                &mut handle,
                descriptor_type,
                0,
                ptr::null_mut(),
            )
        };
        if status != OCI_SUCCESS {
            return Err(DataError::protocol(format!(
                "failed to allocate an OCI descriptor of type {descriptor_type} (status {status})"
            )));
        }
        Ok(handle)
    }

    pub(crate) fn free_descriptor(&self, handle: Handle, descriptor_type: u32) {
        if handle.is_null() {
            return;
        }

        unsafe {
            let _ = (self.oci.functions().descriptor_free)(handle, descriptor_type);
        }
    }

    pub(crate) fn error(&self, error_handle: Handle, status: Sword) -> DataError {
        if error_handle.is_null() {
            return DataError::protocol(format!("OCI call failed with status {status}"));
        }

        let mut code: i16 = 0;
        let mut buffer = [0u8; 1024];

        let _ = unsafe {
            (self.oci.functions().error_get)(
                error_handle,
                1,
                ptr::null_mut(),
                &mut code,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                OCI_HTYPE_ERROR,
            )
        };

        let length = buffer
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(buffer.len());
        let message = String::from_utf8_lossy(&buffer[..length]).trim().to_owned();

        if code == 0 && message.is_empty() {
            return DataError::protocol(format!("OCI call failed with status {status}"));
        }

        DataError::Server {
            // OCI reports the error code through a signed 2-byte `sb2`;
            // widen the raw bit pattern so codes above 32767 (e.g.
            // ORA-65096) keep their true value.
            code: u32::from(code as u16),
            state: "HY000".to_owned(),
            message,
        }
    }

    pub(crate) fn check(&self, status: Sword, error_handle: Handle) -> Result<(), DataError> {
        if status == OCI_SUCCESS || status == OCI_SUCCESS_WITH_INFO {
            Ok(())
        } else {
            Err(self.error(error_handle, status))
        }
    }
}

impl Drop for Environment {
    fn drop(&mut self) {
        self.free(self.handle, OCI_HTYPE_ENV);
    }
}

pub(crate) fn is_numeric_type(data_type: u16) -> bool {
    matches!(
        data_type,
        SQLT_NUM | SQLT_INT | SQLT_FLT | SQLT_VNU | SQLT_BDOUBLE | SQLT_IBFLOAT | SQLT_IBDOUBLE
    )
}

pub(crate) fn is_lob_type(data_type: u16) -> bool {
    matches!(data_type, SQLT_CLOB | SQLT_BLOB)
}
