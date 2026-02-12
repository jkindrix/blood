//! DWARF Debug Information Generation
//!
//! This module generates DWARF debug symbols for Blood programs, enabling
//! source-level debugging with tools like gdb and lldb.
//!
//! # DWARF Standard
//!
//! We generate DWARF 4 debug info (compatible with most debuggers).
//! LLVM's DIBuilder handles the low-level DWARF emission.
//!
//! # Debug Info Components
//!
//! - **Compile Unit**: Top-level container for debug info
//! - **Files**: Source file references
//! - **Subprograms**: Function debug info with line numbers
//! - **Types**: Type definitions for variables
//! - **Variables**: Local variable debug info with locations
//! - **Locations**: Source line/column mappings

use inkwell::context::Context;
use inkwell::debug_info::{
    AsDIScope, DICompileUnit, DIFile, DIFlags, DIFlagsConstants, DILocalVariable, DILocation,
    DIScope, DISubprogram, DISubroutineType, DIType, DWARFEmissionKind,
    DWARFSourceLanguage, DebugInfoBuilder,
};
use inkwell::module::Module;

use std::collections::HashMap;
use std::path::Path;

use crate::hir::{self, DefId, Type, TypeKind, PrimitiveTy};
use crate::span::Span;

/// Debug info builder for a Blood compilation unit.
pub struct DebugInfoGenerator<'ctx> {
    /// The LLVM debug info builder.
    builder: DebugInfoBuilder<'ctx>,
    /// The compile unit for this module.
    compile_unit: DICompileUnit<'ctx>,
    /// Main source file.
    main_file: DIFile<'ctx>,
    /// Cache of source files.
    files: HashMap<String, DIFile<'ctx>>,
    /// Cache of type debug info.
    types: HashMap<TypeCacheKey, DIType<'ctx>>,
    /// Source code for line lookups.
    #[allow(dead_code)]
    source: String,
    /// Current scope stack.
    #[allow(dead_code)]
    scope_stack: Vec<DIScope<'ctx>>,
}

/// Key for the type cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum TypeCacheKey {
    Primitive(PrimitiveTy),
    Pointer,
    Struct(DefId),
    #[allow(dead_code)]
    Enum(DefId),
    Function(Vec<TypeCacheKey>, Box<TypeCacheKey>),
    Tuple(Vec<TypeCacheKey>),
    Array(Box<TypeCacheKey>, u64),
    Ref(Box<TypeCacheKey>, bool), // inner, mutable
    Unit,
    Never,
    Unknown,
}

impl<'ctx> DebugInfoGenerator<'ctx> {
    /// Create a new debug info generator.
    ///
    /// # Arguments
    ///
    /// * `context` - The LLVM context
    /// * `module` - The LLVM module to add debug info to
    /// * `source_path` - Path to the main source file
    /// * `source` - The source code content
    pub fn new(
        _context: &'ctx Context,
        module: &Module<'ctx>,
        source_path: &Path,
        source: &str,
    ) -> Self {
        // Extract directory and filename from path
        let directory = source_path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());
        let filename = source_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown.blood".to_string());

        // Create the debug info builder
        let (builder, compile_unit) = module.create_debug_info_builder(
            true, // allow_unresolved
            DWARFSourceLanguage::C, // Use C for now (Blood isn't in DWARF standard)
            &filename,
            &directory,
            "bloodc", // producer
            false,    // is_optimized
            "",       // flags
            0,        // runtime_version
            "",       // split_name
            DWARFEmissionKind::Full,
            0,    // dwo_id
            true, // split_debug_inlining
            false, // debug_info_for_profiling
            "",   // sys_root
            "",   // sdk
        );

        // Create the main file
        let main_file = builder.create_file(&filename, &directory);

        let mut files = HashMap::new();
        files.insert(source_path.to_string_lossy().to_string(), main_file);

        Self {
            builder,
            compile_unit,
            main_file,
            files,
            types: HashMap::new(),
            source: source.to_string(),
            scope_stack: Vec::new(),
        }
    }

    /// Get or create a DIFile for a source path.
    pub fn get_file(&mut self, path: &Path) -> DIFile<'ctx> {
        let path_str = path.to_string_lossy().to_string();
        if let Some(&file) = self.files.get(&path_str) {
            return file;
        }

        let directory = path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown.blood".to_string());

        let file = self.builder.create_file(&filename, &directory);
        self.files.insert(path_str, file);
        file
    }

    /// Get the main source file.
    pub fn main_file(&self) -> DIFile<'ctx> {
        self.main_file
    }

    /// Get the compile unit scope.
    pub fn compile_unit_scope(&self) -> DIScope<'ctx> {
        self.compile_unit.as_debug_info_scope()
    }

    /// Create debug info for a function.
    ///
    /// Returns the DISubprogram which should be attached to the function.
    pub fn create_function(
        &mut self,
        name: &str,
        linkage_name: Option<&str>,
        file: DIFile<'ctx>,
        line: u32,
        fn_type: DISubroutineType<'ctx>,
        is_local: bool,
        is_definition: bool,
    ) -> DISubprogram<'ctx> {
        let scope = self.compile_unit.as_debug_info_scope();

        self.builder.create_function(
            scope,
            name,
            linkage_name,
            file,
            line,
            fn_type,
            is_local,
            is_definition,
            line, // scope_line
            DIFlags::PUBLIC,
            false, // is_optimized
        )
    }

    /// Create a subroutine type (function signature) for debug info.
    pub fn create_subroutine_type(
        &mut self,
        return_type: Option<DIType<'ctx>>,
        param_types: &[DIType<'ctx>],
    ) -> DISubroutineType<'ctx> {
        self.builder.create_subroutine_type(
            self.main_file,
            return_type,
            param_types,
            DIFlags::PUBLIC,
        )
    }

    /// Create debug info for a function parameter.
    pub fn create_parameter_variable(
        &mut self,
        scope: DIScope<'ctx>,
        name: &str,
        arg_no: u32,
        file: DIFile<'ctx>,
        line: u32,
        ty: DIType<'ctx>,
    ) -> DILocalVariable<'ctx> {
        self.builder.create_parameter_variable(
            scope,
            name,
            arg_no,
            file,
            line,
            ty,
            true, // always_preserve
            DIFlags::ZERO,
        )
    }

    /// Create debug info for a local variable.
    pub fn create_local_variable(
        &mut self,
        scope: DIScope<'ctx>,
        name: &str,
        file: DIFile<'ctx>,
        line: u32,
        ty: DIType<'ctx>,
    ) -> DILocalVariable<'ctx> {
        self.builder.create_auto_variable(
            scope,
            name,
            file,
            line,
            ty,
            true, // always_preserve
            DIFlags::ZERO,
            0, // align_in_bits
        )
    }

    /// Create a debug location for source mapping.
    pub fn create_location(
        &self,
        context: &'ctx Context,
        line: u32,
        column: u32,
        scope: DIScope<'ctx>,
    ) -> DILocation<'ctx> {
        self.builder
            .create_debug_location(context, line, column, scope, None)
    }

    /// Convert a span to line number.
    pub fn span_to_line(&self, span: Span) -> u32 {
        span.start_line
    }

    /// Convert a span to column number.
    pub fn span_to_column(&self, span: Span) -> u32 {
        span.start_col
    }

    /// Get or create debug type for a HIR type.
    pub fn get_type(&mut self, ty: &Type) -> Option<DIType<'ctx>> {
        let key = self.type_to_key(ty);
        if let Some(&cached) = self.types.get(&key) {
            return Some(cached);
        }

        let di_type = self.create_type(ty)?;
        self.types.insert(key, di_type);
        Some(di_type)
    }

    /// Create a cache key for a type.
    fn type_to_key(&self, ty: &Type) -> TypeCacheKey {
        match ty.kind() {
            TypeKind::Primitive(p) => TypeCacheKey::Primitive(*p),
            TypeKind::Tuple(elems) if elems.is_empty() => TypeCacheKey::Unit,
            TypeKind::Tuple(elems) => {
                TypeCacheKey::Tuple(elems.iter().map(|e| self.type_to_key(e)).collect())
            }
            TypeKind::Array { element, size } => {
                let size_val: u64 = match size {
                    hir::ConstValue::Uint(n) => (*n).try_into().unwrap_or(u64::MAX),
                    hir::ConstValue::Int(n) => (*n).max(0) as u64,
                    _ => 0,
                };
                TypeCacheKey::Array(Box::new(self.type_to_key(element)), size_val)
            }
            TypeKind::Ref { inner, mutable } => {
                TypeCacheKey::Ref(Box::new(self.type_to_key(inner)), *mutable)
            }
            TypeKind::Ptr { .. } => TypeCacheKey::Pointer,
            TypeKind::Adt { def_id, .. } => TypeCacheKey::Struct(*def_id),
            TypeKind::Never => TypeCacheKey::Never,
            TypeKind::Fn { params, ret, .. } => TypeCacheKey::Function(
                params.iter().map(|p| self.type_to_key(p)).collect(),
                Box::new(self.type_to_key(ret)),
            ),
            _ => TypeCacheKey::Unknown,
        }
    }

    /// Create a debug type for a HIR type.
    fn create_type(&mut self, ty: &Type) -> Option<DIType<'ctx>> {
        match ty.kind() {
            TypeKind::Primitive(p) => self.create_primitive_type(p),
            TypeKind::Tuple(elems) if elems.is_empty() => {
                // Unit type - void
                None
            }
            TypeKind::Tuple(elems) => {
                // Tuple as anonymous struct
                let elem_types: Vec<_> = elems.iter().filter_map(|e| self.get_type(e)).collect();
                // Calculate size (simplified - assume 8-byte alignment)
                let size_bits = elem_types.len() as u64 * 64;
                Some(
                    self.builder
                        .create_struct_type(
                            self.compile_unit.as_debug_info_scope(),
                            "()",
                            self.main_file,
                            0,
                            size_bits,
                            64,
                            DIFlags::ZERO,
                            None,
                            &elem_types,
                            0,
                            None,
                            "",
                        )
                        .as_type(),
                )
            }
            TypeKind::Ref { inner, mutable: _ } | TypeKind::Ptr { inner, mutable: _ } => {
                let pointee = self.get_type(inner);
                let pointee_type = match pointee {
                    Some(t) => t,
                    None => self.create_void_type(),
                };
                Some(
                    self.builder
                        .create_pointer_type(
                            "&",
                            pointee_type,
                            64, // 64-bit pointers
                            64, // alignment
                            inkwell::AddressSpace::default(),
                        )
                        .as_type(),
                )
            }
            TypeKind::Array { element, size } => {
                let elem_type = self.get_type(element)?;
                let size_val: u64 = match size {
                    hir::ConstValue::Uint(n) => (*n).try_into().unwrap_or(u64::MAX),
                    hir::ConstValue::Int(n) => (*n).max(0) as u64,
                    _ => 0,
                };
                // Create array type
                // Note: inkwell doesn't have create_array_type, use struct as workaround
                let size_bits = size_val * 64; // Simplified
                Some(
                    self.builder
                        .create_struct_type(
                            self.compile_unit.as_debug_info_scope(),
                            &format!("[{}]", size_val),
                            self.main_file,
                            0,
                            size_bits,
                            64,
                            DIFlags::ZERO,
                            None,
                            &[elem_type],
                            0,
                            None,
                            "",
                        )
                        .as_type(),
                )
            }
            TypeKind::Never => {
                // Never type - use void
                None
            }
            _ => {
                // Unknown types - use i8 as placeholder
                self.builder
                    .create_basic_type("unknown", 8, 0x07, DIFlags::ZERO)
                    .ok()
                    .map(|t| t.as_type())
            }
        }
    }

    /// Create debug type for a primitive.
    fn create_primitive_type(&mut self, prim: &PrimitiveTy) -> Option<DIType<'ctx>> {
        let (name, size_bits, encoding) = match prim {
            PrimitiveTy::Int(int_ty) => {
                let (name, size) = match int_ty {
                    hir::IntTy::I8 => ("i8", 8),
                    hir::IntTy::I16 => ("i16", 16),
                    hir::IntTy::I32 => ("i32", 32),
                    hir::IntTy::I64 => ("i64", 64),
                    hir::IntTy::I128 => ("i128", 128),
                    hir::IntTy::Isize => ("isize", 64),
                };
                (name, size, 0x05) // DW_ATE_signed
            }
            PrimitiveTy::Uint(uint_ty) => {
                let (name, size) = match uint_ty {
                    hir::UintTy::U8 => ("u8", 8),
                    hir::UintTy::U16 => ("u16", 16),
                    hir::UintTy::U32 => ("u32", 32),
                    hir::UintTy::U64 => ("u64", 64),
                    hir::UintTy::U128 => ("u128", 128),
                    hir::UintTy::Usize => ("usize", 64),
                };
                (name, size, 0x07) // DW_ATE_unsigned
            }
            PrimitiveTy::Float(float_ty) => {
                let (name, size) = match float_ty {
                    hir::FloatTy::F32 => ("f32", 32),
                    hir::FloatTy::F64 => ("f64", 64),
                };
                (name, size, 0x04) // DW_ATE_float
            }
            PrimitiveTy::Bool => ("bool", 8, 0x02), // DW_ATE_boolean
            PrimitiveTy::Char => ("char", 32, 0x08), // DW_ATE_unsigned_char (UTF-32)
            PrimitiveTy::Str => {
                // String slice - pointer + length
                return self
                    .builder
                    .create_basic_type("str", 128, 0x07, DIFlags::ZERO)
                    .ok()
                    .map(|t| t.as_type());
            }
            PrimitiveTy::String => {
                // Owned string - pointer + length + capacity
                return self
                    .builder
                    .create_basic_type("String", 192, 0x07, DIFlags::ZERO)
                    .ok()
                    .map(|t| t.as_type());
            }
            PrimitiveTy::Unit => return None,
            PrimitiveTy::Never => return None,
        };

        self.builder
            .create_basic_type(name, size_bits, encoding, DIFlags::ZERO)
            .ok()
            .map(|t| t.as_type())
    }

    /// Create a void type for pointers to unknown types.
    fn create_void_type(&mut self) -> DIType<'ctx> {
        self.builder
            .create_basic_type("void", 0, 0x00, DIFlags::ZERO)
            .unwrap()
            .as_type()
    }

    /// Finalize debug info generation.
    ///
    /// This must be called after all debug info has been added.
    pub fn finalize(&self) {
        self.builder.finalize();
    }

    /// Insert a dbg.declare intrinsic for a local variable.
    pub fn insert_declare(
        &self,
        _builder: &inkwell::builder::Builder<'ctx>,
        storage: inkwell::values::PointerValue<'ctx>,
        var_info: DILocalVariable<'ctx>,
        location: DILocation<'ctx>,
        insert_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) {
        self.builder.insert_declare_at_end(
            storage,
            Some(var_info),
            None, // expression
            location,
            insert_block,
        );
    }
}

/// Configuration for debug info generation.
#[derive(Debug, Clone, Default)]
pub struct DebugInfoConfig {
    /// Whether to generate debug info.
    pub enabled: bool,
    /// Optimization level affects debug info quality.
    pub optimized: bool,
    /// Include column information (can increase debug info size).
    pub include_columns: bool,
}

impl DebugInfoConfig {
    /// Create config for debug builds.
    pub fn debug() -> Self {
        Self {
            enabled: true,
            optimized: false,
            include_columns: true,
        }
    }

    /// Create config for release builds.
    pub fn release() -> Self {
        Self {
            enabled: false,
            optimized: true,
            include_columns: false,
        }
    }

    /// Create config for release with debug info.
    pub fn release_with_debug() -> Self {
        Self {
            enabled: true,
            optimized: true,
            include_columns: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_cache_key() {
        // Ensure cache keys work correctly
        let key1 = TypeCacheKey::Primitive(PrimitiveTy::Int(hir::IntTy::I32));
        let key2 = TypeCacheKey::Primitive(PrimitiveTy::Int(hir::IntTy::I32));
        assert_eq!(key1, key2);

        let key3 = TypeCacheKey::Primitive(PrimitiveTy::Int(hir::IntTy::I64));
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_debug_info_config() {
        let debug = DebugInfoConfig::debug();
        assert!(debug.enabled);
        assert!(!debug.optimized);

        let release = DebugInfoConfig::release();
        assert!(!release.enabled);
        assert!(release.optimized);
    }
}
