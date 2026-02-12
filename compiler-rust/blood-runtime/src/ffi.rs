//! # Foreign Function Interface (FFI)
//!
//! Safe FFI support for calling external functions.
//!
//! ## Design
//!
//! Based on the Blood runtime specification (ROADMAP.md §9.4):
//! - Dynamic library loading via libloading
//! - Type-safe argument marshaling
//! - Rust 2024 Edition FFI conventions
//!
//! ## Technical References
//!
//! - [libloading](https://docs.rs/libloading)
//! - [Rustonomicon FFI](https://doc.rust-lang.org/nomicon/ffi.html)
//! - [Rust 2024 Edition FFI](https://doc.rust-lang.org/edition-guide/rust-2024/unsafe-extern.html)

use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::fmt;
use std::os::raw::c_char;
use std::path::Path;
use std::sync::Arc;

use libloading::{Library, Symbol};
use parking_lot::RwLock;

/// FFI error.
#[derive(Debug, Clone)]
pub struct FfiError {
    /// Error kind.
    pub kind: FfiErrorKind,
    /// Error message.
    pub message: String,
}

impl FfiError {
    /// Create a new FFI error.
    pub fn new(kind: FfiErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for FfiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for FfiError {}

/// FFI error kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiErrorKind {
    /// Library not found.
    LibraryNotFound,
    /// Symbol not found.
    SymbolNotFound,
    /// Type mismatch during marshaling.
    TypeMismatch,
    /// Null pointer.
    NullPointer,
    /// Invalid UTF-8 string.
    InvalidUtf8,
    /// Library already loaded.
    AlreadyLoaded,
    /// Call failed.
    CallFailed,
}

/// FFI type for argument/return marshaling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiType {
    /// Void (no value).
    Void,
    /// 8-bit signed integer.
    I8,
    /// 16-bit signed integer.
    I16,
    /// 32-bit signed integer.
    I32,
    /// 64-bit signed integer.
    I64,
    /// 8-bit unsigned integer.
    U8,
    /// 16-bit unsigned integer.
    U16,
    /// 32-bit unsigned integer.
    U32,
    /// 64-bit unsigned integer.
    U64,
    /// 32-bit floating point.
    F32,
    /// 64-bit floating point.
    F64,
    /// Pointer (void*).
    Pointer,
    /// C string (char*).
    CString,
    /// Boolean (as i32).
    Bool,
    /// Size type (usize).
    Size,
}

impl FfiType {
    /// Get the size of this type in bytes.
    pub fn size(&self) -> usize {
        match self {
            FfiType::Void => 0,
            FfiType::I8 | FfiType::U8 | FfiType::Bool => 1,
            FfiType::I16 | FfiType::U16 => 2,
            FfiType::I32 | FfiType::U32 | FfiType::F32 => 4,
            FfiType::I64 | FfiType::U64 | FfiType::F64 => 8,
            FfiType::Pointer | FfiType::CString | FfiType::Size => std::mem::size_of::<usize>(),
        }
    }

    /// Get the alignment of this type.
    pub fn alignment(&self) -> usize {
        self.size().max(1)
    }
}

/// FFI value for passing to/from foreign functions.
#[derive(Debug, Clone)]
pub enum FfiValue {
    /// Void (no value).
    Void,
    /// 8-bit signed integer.
    I8(i8),
    /// 16-bit signed integer.
    I16(i16),
    /// 32-bit signed integer.
    I32(i32),
    /// 64-bit signed integer.
    I64(i64),
    /// 8-bit unsigned integer.
    U8(u8),
    /// 16-bit unsigned integer.
    U16(u16),
    /// 32-bit unsigned integer.
    U32(u32),
    /// 64-bit unsigned integer.
    U64(u64),
    /// 32-bit floating point.
    F32(f32),
    /// 64-bit floating point.
    F64(f64),
    /// Raw pointer.
    Pointer(*mut c_void),
    /// String value (owned).
    String(String),
    /// Boolean.
    Bool(bool),
    /// Size.
    Size(usize),
}

impl FfiValue {
    /// Get the type of this value.
    pub fn ffi_type(&self) -> FfiType {
        match self {
            FfiValue::Void => FfiType::Void,
            FfiValue::I8(_) => FfiType::I8,
            FfiValue::I16(_) => FfiType::I16,
            FfiValue::I32(_) => FfiType::I32,
            FfiValue::I64(_) => FfiType::I64,
            FfiValue::U8(_) => FfiType::U8,
            FfiValue::U16(_) => FfiType::U16,
            FfiValue::U32(_) => FfiType::U32,
            FfiValue::U64(_) => FfiType::U64,
            FfiValue::F32(_) => FfiType::F32,
            FfiValue::F64(_) => FfiType::F64,
            FfiValue::Pointer(_) => FfiType::Pointer,
            FfiValue::String(_) => FfiType::CString,
            FfiValue::Bool(_) => FfiType::Bool,
            FfiValue::Size(_) => FfiType::Size,
        }
    }

    /// Try to convert to i32.
    pub fn as_i32(&self) -> Option<i32> {
        match self {
            FfiValue::I8(v) => Some(*v as i32),
            FfiValue::I16(v) => Some(*v as i32),
            FfiValue::I32(v) => Some(*v),
            FfiValue::U8(v) => Some(*v as i32),
            FfiValue::U16(v) => Some(*v as i32),
            FfiValue::Bool(v) => Some(if *v { 1 } else { 0 }),
            _ => None,
        }
    }

    /// Try to convert to i64.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            FfiValue::I8(v) => Some(*v as i64),
            FfiValue::I16(v) => Some(*v as i64),
            FfiValue::I32(v) => Some(*v as i64),
            FfiValue::I64(v) => Some(*v),
            FfiValue::U8(v) => Some(*v as i64),
            FfiValue::U16(v) => Some(*v as i64),
            FfiValue::U32(v) => Some(*v as i64),
            FfiValue::Size(v) => Some(*v as i64),
            FfiValue::Bool(v) => Some(if *v { 1 } else { 0 }),
            _ => None,
        }
    }

    /// Try to convert to f64.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            FfiValue::F32(v) => Some(*v as f64),
            FfiValue::F64(v) => Some(*v),
            _ => None,
        }
    }

    /// Try to convert to pointer.
    pub fn as_pointer(&self) -> Option<*mut c_void> {
        match self {
            FfiValue::Pointer(p) => Some(*p),
            _ => None,
        }
    }
}

// Safety: FfiValue contains raw pointers but they are only used within
// the FFI context where the caller is responsible for safety.
unsafe impl Send for FfiValue {}
unsafe impl Sync for FfiValue {}

/// Foreign function signature.
#[derive(Debug, Clone)]
pub struct FfiSignature {
    /// Parameter types.
    pub params: Vec<FfiType>,
    /// Return type.
    pub return_type: FfiType,
    /// Whether the function uses varargs.
    pub varargs: bool,
}

impl FfiSignature {
    /// Create a new signature.
    pub fn new(params: Vec<FfiType>, return_type: FfiType) -> Self {
        Self {
            params,
            return_type,
            varargs: false,
        }
    }

    /// Create a varargs signature.
    pub fn varargs(params: Vec<FfiType>, return_type: FfiType) -> Self {
        Self {
            params,
            return_type,
            varargs: true,
        }
    }
}

/// Handle to a loaded dynamic library.
pub struct DynamicLibrary {
    /// The library name/path.
    name: String,
    /// The loaded library.
    library: Library,
}

impl DynamicLibrary {
    /// Load a dynamic library.
    ///
    /// # Safety
    ///
    /// Loading a library can execute initialization code. The caller must
    /// ensure the library is trusted.
    pub unsafe fn load(path: impl AsRef<Path>) -> Result<Self, FfiError> {
        let path = path.as_ref();
        let library = Library::new(path).map_err(|e| {
            FfiError::new(
                FfiErrorKind::LibraryNotFound,
                format!("failed to load library '{}': {}", path.display(), e),
            )
        })?;

        Ok(Self {
            name: path.display().to_string(),
            library,
        })
    }

    /// Get the library name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get a symbol from the library.
    ///
    /// # Safety
    ///
    /// The caller must ensure the symbol has the correct type signature.
    pub unsafe fn get_symbol<T>(&self, name: &str) -> Result<Symbol<'_, T>, FfiError> {
        let cname = CString::new(name).map_err(|_| {
            FfiError::new(FfiErrorKind::InvalidUtf8, "symbol name contains null byte")
        })?;

        self.library.get(cname.as_bytes_with_nul()).map_err(|e| {
            FfiError::new(
                FfiErrorKind::SymbolNotFound,
                format!("symbol '{}' not found: {}", name, e),
            )
        })
    }

    /// Check if a symbol exists.
    pub fn has_symbol(&self, name: &str) -> bool {
        let cname = match CString::new(name) {
            Ok(c) => c,
            Err(_) => return false,
        };

        unsafe {
            self.library
                .get::<*const c_void>(cname.as_bytes_with_nul())
                .is_ok()
        }
    }
}

impl fmt::Debug for DynamicLibrary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DynamicLibrary")
            .field("name", &self.name)
            .finish()
    }
}

/// Registry of loaded libraries.
pub struct LibraryRegistry {
    /// Loaded libraries.
    libraries: RwLock<HashMap<String, Arc<DynamicLibrary>>>,
}

impl LibraryRegistry {
    /// Create a new library registry.
    pub fn new() -> Self {
        Self {
            libraries: RwLock::new(HashMap::new()),
        }
    }

    /// Load a library into the registry.
    ///
    /// # Safety
    ///
    /// Loading a library can execute initialization code.
    pub unsafe fn load(&self, path: impl AsRef<Path>) -> Result<Arc<DynamicLibrary>, FfiError> {
        let path = path.as_ref();
        let key = path.display().to_string();

        // Check if already loaded
        {
            let libs = self.libraries.read();
            if let Some(lib) = libs.get(&key) {
                return Ok(lib.clone());
            }
        }

        // Load the library
        let lib = Arc::new(DynamicLibrary::load(path)?);

        // Store in registry
        {
            let mut libs = self.libraries.write();
            libs.insert(key, lib.clone());
        }

        Ok(lib)
    }

    /// Get a loaded library.
    pub fn get(&self, name: &str) -> Option<Arc<DynamicLibrary>> {
        self.libraries.read().get(name).cloned()
    }

    /// Check if a library is loaded.
    pub fn is_loaded(&self, name: &str) -> bool {
        self.libraries.read().contains_key(name)
    }

    /// Unload a library.
    pub fn unload(&self, name: &str) -> bool {
        self.libraries.write().remove(name).is_some()
    }

    /// Get the number of loaded libraries.
    pub fn count(&self) -> usize {
        self.libraries.read().len()
    }
}

impl Default for LibraryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for FFI result.
pub type FfiResult<T> = Result<T, FfiError>;

// ============================================================================
// Helper functions
// ============================================================================

/// Convert a Rust string to a C string.
pub fn to_cstring(s: &str) -> FfiResult<CString> {
    CString::new(s).map_err(|_| {
        FfiError::new(FfiErrorKind::InvalidUtf8, "string contains null byte")
    })
}

/// Convert a C string to a Rust string.
///
/// # Safety
///
/// The pointer must point to a valid null-terminated C string.
pub unsafe fn from_cstring(ptr: *const c_char) -> FfiResult<String> {
    if ptr.is_null() {
        return Err(FfiError::new(FfiErrorKind::NullPointer, "null pointer"));
    }

    CStr::from_ptr(ptr)
        .to_str()
        .map(|s| s.to_string())
        .map_err(|_| FfiError::new(FfiErrorKind::InvalidUtf8, "invalid UTF-8 string"))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ffi_type_size() {
        assert_eq!(FfiType::Void.size(), 0);
        assert_eq!(FfiType::I8.size(), 1);
        assert_eq!(FfiType::I16.size(), 2);
        assert_eq!(FfiType::I32.size(), 4);
        assert_eq!(FfiType::I64.size(), 8);
        assert_eq!(FfiType::F32.size(), 4);
        assert_eq!(FfiType::F64.size(), 8);
    }

    #[test]
    fn test_ffi_value_type() {
        assert_eq!(FfiValue::Void.ffi_type(), FfiType::Void);
        assert_eq!(FfiValue::I32(42).ffi_type(), FfiType::I32);
        assert_eq!(FfiValue::F64(2.5).ffi_type(), FfiType::F64);
        assert_eq!(FfiValue::Bool(true).ffi_type(), FfiType::Bool);
    }

    #[test]
    fn test_ffi_value_conversions() {
        assert_eq!(FfiValue::I32(42).as_i32(), Some(42));
        assert_eq!(FfiValue::I64(100).as_i64(), Some(100));
        assert_eq!(FfiValue::F64(2.5).as_f64(), Some(2.5));
        assert_eq!(FfiValue::Bool(true).as_i32(), Some(1));
        assert_eq!(FfiValue::Bool(false).as_i32(), Some(0));
    }

    #[test]
    fn test_ffi_signature() {
        let sig = FfiSignature::new(
            vec![FfiType::I32, FfiType::Pointer],
            FfiType::I32,
        );
        assert_eq!(sig.params.len(), 2);
        assert_eq!(sig.return_type, FfiType::I32);
        assert!(!sig.varargs);

        let varargs_sig = FfiSignature::varargs(
            vec![FfiType::CString],
            FfiType::I32,
        );
        assert!(varargs_sig.varargs);
    }

    #[test]
    fn test_to_cstring() {
        let s = to_cstring("hello").unwrap();
        assert_eq!(s.as_bytes(), b"hello");

        let err = to_cstring("hel\0lo");
        assert!(err.is_err());
    }

    #[test]
    fn test_from_cstring() {
        let s = CString::new("hello").unwrap();
        let result = unsafe { from_cstring(s.as_ptr()) };
        assert_eq!(result.unwrap(), "hello");

        let null_result = unsafe { from_cstring(std::ptr::null()) };
        assert!(null_result.is_err());
    }

    #[test]
    fn test_library_registry() {
        let registry = LibraryRegistry::new();
        assert_eq!(registry.count(), 0);
        assert!(!registry.is_loaded("nonexistent"));
    }

    #[test]
    fn test_ffi_error() {
        let err = FfiError::new(FfiErrorKind::SymbolNotFound, "test symbol");
        assert_eq!(err.kind, FfiErrorKind::SymbolNotFound);
        assert!(err.to_string().contains("test symbol"));
    }
}
