//! Grammar-based generators for Blood source code.
//!
//! This module implements the full Blood grammar as described in GRAMMAR.md,
//! generating syntactically plausible source code for fuzzing.

use arbitrary::{Arbitrary, Unstructured};
use crate::ident::{FuzzIdent, FuzzTypeIdent};

// ============================================================
// Constants for controlling generation complexity
// ============================================================

/// Maximum depth for recursive structures (expressions, types, etc.)
const MAX_DEPTH: u8 = 5;

/// Maximum number of items in lists (parameters, fields, etc.)
const MAX_LIST_LEN: u8 = 6;

// ============================================================
// Program Structure
// ============================================================

/// A complete Blood program.
#[derive(Debug, Clone)]
pub struct FuzzProgram {
    pub declarations: Vec<FuzzDeclaration>,
}

impl FuzzProgram {
    pub fn to_source(&self) -> String {
        self.declarations
            .iter()
            .map(|d| d.to_source())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

impl<'a> Arbitrary<'a> for FuzzProgram {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let count: u8 = u.arbitrary()?;
        let count = (count % 5) + 1; // 1-5 declarations

        let mut declarations = Vec::with_capacity(count as usize);
        for _ in 0..count {
            declarations.push(FuzzDeclaration::arbitrary_with_depth(u, 0)?);
        }

        Ok(FuzzProgram { declarations })
    }
}

// ============================================================
// Declarations
// ============================================================

/// A top-level declaration.
#[derive(Debug, Clone)]
pub enum FuzzDeclaration {
    Function(FuzzFunction),
    Struct(FuzzStruct),
    Enum(FuzzEnum),
    Effect(FuzzEffect),
    Handler(FuzzHandler),
    TypeAlias(FuzzTypeAlias),
    Const(FuzzConst),
}

impl FuzzDeclaration {
    pub fn to_source(&self) -> String {
        match self {
            FuzzDeclaration::Function(f) => f.to_source(),
            FuzzDeclaration::Struct(s) => s.to_source(),
            FuzzDeclaration::Enum(e) => e.to_source(),
            FuzzDeclaration::Effect(e) => e.to_source(),
            FuzzDeclaration::Handler(h) => h.to_source(),
            FuzzDeclaration::TypeAlias(t) => t.to_source(),
            FuzzDeclaration::Const(c) => c.to_source(),
        }
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        let choice: u8 = u.arbitrary()?;
        Ok(match choice % 7 {
            0 => FuzzDeclaration::Function(FuzzFunction::arbitrary_with_depth(u, depth)?),
            1 => FuzzDeclaration::Struct(FuzzStruct::arbitrary(u)?),
            2 => FuzzDeclaration::Enum(FuzzEnum::arbitrary(u)?),
            3 => FuzzDeclaration::Effect(FuzzEffect::arbitrary(u)?),
            4 => FuzzDeclaration::Handler(FuzzHandler::arbitrary_with_depth(u, depth)?),
            5 => FuzzDeclaration::TypeAlias(FuzzTypeAlias::arbitrary_with_depth(u, depth)?),
            _ => FuzzDeclaration::Const(FuzzConst::arbitrary_with_depth(u, depth)?),
        })
    }
}

impl<'a> Arbitrary<'a> for FuzzDeclaration {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Self::arbitrary_with_depth(u, 0)
    }
}

// ============================================================
// Function Declaration
// ============================================================

/// A function declaration.
#[derive(Debug, Clone)]
pub struct FuzzFunction {
    pub visibility: FuzzVisibility,
    pub name: FuzzIdent,
    pub type_params: Vec<FuzzTypeParam>,
    pub params: Vec<FuzzParam>,
    pub return_type: Option<FuzzType>,
    pub effect_row: Option<FuzzEffectRow>,
    pub body: Option<FuzzBlock>,
}

impl FuzzFunction {
    pub fn to_source(&self) -> String {
        let mut s = String::new();

        // Visibility
        s.push_str(&self.visibility.to_source());
        if !s.is_empty() {
            s.push(' ');
        }

        s.push_str("fn ");
        s.push_str(&self.name.to_source());

        // Type parameters
        if !self.type_params.is_empty() {
            s.push('<');
            s.push_str(
                &self
                    .type_params
                    .iter()
                    .map(|p| p.to_source())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            s.push('>');
        }

        // Parameters
        s.push('(');
        s.push_str(
            &self
                .params
                .iter()
                .map(|p| p.to_source())
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push(')');

        // Return type
        if let Some(ret) = &self.return_type {
            s.push_str(" -> ");
            s.push_str(&ret.to_source());
        }

        // Effect row
        if let Some(effects) = &self.effect_row {
            s.push_str(" / ");
            s.push_str(&effects.to_source());
        }

        // Body
        if let Some(body) = &self.body {
            s.push(' ');
            s.push_str(&body.to_source());
        } else {
            s.push(';');
        }

        s
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        let visibility = FuzzVisibility::arbitrary(u)?;
        let name = FuzzIdent::arbitrary(u)?;

        let type_param_count: u8 = u.arbitrary()?;
        let type_param_count = type_param_count % 3;
        let mut type_params = Vec::with_capacity(type_param_count as usize);
        for _ in 0..type_param_count {
            type_params.push(FuzzTypeParam::arbitrary_with_depth(u, depth)?);
        }

        let param_count: u8 = u.arbitrary()?;
        let param_count = param_count % MAX_LIST_LEN;
        let mut params = Vec::with_capacity(param_count as usize);
        for _ in 0..param_count {
            params.push(FuzzParam::arbitrary_with_depth(u, depth)?);
        }

        let has_return: bool = u.arbitrary()?;
        let return_type = if has_return {
            Some(FuzzType::arbitrary_with_depth(u, depth)?)
        } else {
            None
        };

        let has_effects: bool = u.arbitrary()?;
        let effect_row = if has_effects {
            Some(FuzzEffectRow::arbitrary(u)?)
        } else {
            None
        };

        let has_body: bool = u.arbitrary()?;
        let body = if has_body {
            Some(FuzzBlock::arbitrary_with_depth(u, depth)?)
        } else {
            None
        };

        Ok(FuzzFunction {
            visibility,
            name,
            type_params,
            params,
            return_type,
            effect_row,
            body,
        })
    }
}

impl<'a> Arbitrary<'a> for FuzzFunction {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Self::arbitrary_with_depth(u, 0)
    }
}

// ============================================================
// Struct Declaration
// ============================================================

/// A struct declaration.
#[derive(Debug, Clone)]
pub struct FuzzStruct {
    pub visibility: FuzzVisibility,
    pub name: FuzzTypeIdent,
    pub type_params: Vec<FuzzTypeParam>,
    pub body: FuzzStructBody,
}

impl FuzzStruct {
    pub fn to_source(&self) -> String {
        let mut s = String::new();

        s.push_str(&self.visibility.to_source());
        if !s.is_empty() {
            s.push(' ');
        }

        s.push_str("struct ");
        s.push_str(&self.name.to_source());

        if !self.type_params.is_empty() {
            s.push('<');
            s.push_str(
                &self
                    .type_params
                    .iter()
                    .map(|p| p.to_source())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            s.push('>');
        }

        s.push_str(&self.body.to_source());
        s
    }
}

impl<'a> Arbitrary<'a> for FuzzStruct {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(FuzzStruct {
            visibility: FuzzVisibility::arbitrary(u)?,
            name: FuzzTypeIdent::arbitrary(u)?,
            type_params: {
                let count: u8 = u.arbitrary()?;
                let count = count % 3;
                let mut params = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    params.push(FuzzTypeParam::arbitrary_with_depth(u, 0)?);
                }
                params
            },
            body: FuzzStructBody::arbitrary(u)?,
        })
    }
}

/// Struct body variants.
#[derive(Debug, Clone)]
pub enum FuzzStructBody {
    /// Named fields: `{ field: Type, ... }`
    Named(Vec<FuzzStructField>),
    /// Tuple fields: `(Type, ...);`
    Tuple(Vec<FuzzType>),
    /// Unit struct: `;`
    Unit,
}

impl FuzzStructBody {
    pub fn to_source(&self) -> String {
        match self {
            FuzzStructBody::Named(fields) => {
                let mut s = String::from(" {\n");
                for field in fields {
                    s.push_str("    ");
                    s.push_str(&field.to_source());
                    s.push_str(",\n");
                }
                s.push('}');
                s
            }
            FuzzStructBody::Tuple(types) => {
                let mut s = String::from("(");
                s.push_str(
                    &types
                        .iter()
                        .map(|t| t.to_source())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                s.push_str(");");
                s
            }
            FuzzStructBody::Unit => ";".to_string(),
        }
    }
}

impl<'a> Arbitrary<'a> for FuzzStructBody {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let choice: u8 = u.arbitrary()?;
        Ok(match choice % 3 {
            0 => {
                let count: u8 = u.arbitrary()?;
                let count = count % MAX_LIST_LEN;
                let mut fields = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    fields.push(FuzzStructField::arbitrary(u)?);
                }
                FuzzStructBody::Named(fields)
            }
            1 => {
                let count: u8 = u.arbitrary()?;
                let count = (count % MAX_LIST_LEN) + 1;
                let mut types = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    types.push(FuzzType::arbitrary_with_depth(u, 0)?);
                }
                FuzzStructBody::Tuple(types)
            }
            _ => FuzzStructBody::Unit,
        })
    }
}

/// A struct field.
#[derive(Debug, Clone)]
pub struct FuzzStructField {
    pub visibility: FuzzVisibility,
    pub name: FuzzIdent,
    pub ty: FuzzType,
}

impl FuzzStructField {
    pub fn to_source(&self) -> String {
        let mut s = String::new();
        s.push_str(&self.visibility.to_source());
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&self.name.to_source());
        s.push_str(": ");
        s.push_str(&self.ty.to_source());
        s
    }
}

impl<'a> Arbitrary<'a> for FuzzStructField {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(FuzzStructField {
            visibility: FuzzVisibility::arbitrary(u)?,
            name: FuzzIdent::arbitrary(u)?,
            ty: FuzzType::arbitrary_with_depth(u, 0)?,
        })
    }
}

// ============================================================
// Enum Declaration
// ============================================================

/// An enum declaration.
#[derive(Debug, Clone)]
pub struct FuzzEnum {
    pub visibility: FuzzVisibility,
    pub name: FuzzTypeIdent,
    pub type_params: Vec<FuzzTypeParam>,
    pub variants: Vec<FuzzEnumVariant>,
}

impl FuzzEnum {
    pub fn to_source(&self) -> String {
        let mut s = String::new();

        s.push_str(&self.visibility.to_source());
        if !s.is_empty() {
            s.push(' ');
        }

        s.push_str("enum ");
        s.push_str(&self.name.to_source());

        if !self.type_params.is_empty() {
            s.push('<');
            s.push_str(
                &self
                    .type_params
                    .iter()
                    .map(|p| p.to_source())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            s.push('>');
        }

        s.push_str(" {\n");
        for variant in &self.variants {
            s.push_str("    ");
            s.push_str(&variant.to_source());
            s.push_str(",\n");
        }
        s.push('}');
        s
    }
}

impl<'a> Arbitrary<'a> for FuzzEnum {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let count: u8 = u.arbitrary()?;
        let count = (count % MAX_LIST_LEN) + 1; // At least 1 variant

        let mut variants = Vec::with_capacity(count as usize);
        for _ in 0..count {
            variants.push(FuzzEnumVariant::arbitrary(u)?);
        }

        Ok(FuzzEnum {
            visibility: FuzzVisibility::arbitrary(u)?,
            name: FuzzTypeIdent::arbitrary(u)?,
            type_params: {
                let count: u8 = u.arbitrary()?;
                let count = count % 3;
                let mut params = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    params.push(FuzzTypeParam::arbitrary_with_depth(u, 0)?);
                }
                params
            },
            variants,
        })
    }
}

/// An enum variant.
#[derive(Debug, Clone)]
pub struct FuzzEnumVariant {
    pub name: FuzzTypeIdent,
    pub body: Option<FuzzStructBody>,
}

impl FuzzEnumVariant {
    pub fn to_source(&self) -> String {
        let mut s = self.name.to_source();
        if let Some(body) = &self.body {
            // Remove trailing semicolon for unit/tuple variants in enum context
            let body_src = body.to_source();
            if body_src.ends_with(';') {
                s.push_str(&body_src[..body_src.len() - 1]);
            } else {
                s.push_str(&body_src);
            }
        }
        s
    }
}

impl<'a> Arbitrary<'a> for FuzzEnumVariant {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let has_body: bool = u.arbitrary()?;
        Ok(FuzzEnumVariant {
            name: FuzzTypeIdent::arbitrary(u)?,
            body: if has_body {
                Some(FuzzStructBody::arbitrary(u)?)
            } else {
                None
            },
        })
    }
}

// ============================================================
// Effect Declaration
// ============================================================

/// An effect declaration.
#[derive(Debug, Clone)]
pub struct FuzzEffect {
    pub name: FuzzTypeIdent,
    pub type_params: Vec<FuzzTypeParam>,
    pub operations: Vec<FuzzOperation>,
}

impl FuzzEffect {
    pub fn to_source(&self) -> String {
        let mut s = String::from("effect ");
        s.push_str(&self.name.to_source());

        if !self.type_params.is_empty() {
            s.push('<');
            s.push_str(
                &self
                    .type_params
                    .iter()
                    .map(|p| p.to_source())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            s.push('>');
        }

        s.push_str(" {\n");
        for op in &self.operations {
            s.push_str("    ");
            s.push_str(&op.to_source());
            s.push('\n');
        }
        s.push('}');
        s
    }
}

impl<'a> Arbitrary<'a> for FuzzEffect {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let op_count: u8 = u.arbitrary()?;
        let op_count = (op_count % 4) + 1; // 1-4 operations

        let mut operations = Vec::with_capacity(op_count as usize);
        for _ in 0..op_count {
            operations.push(FuzzOperation::arbitrary(u)?);
        }

        Ok(FuzzEffect {
            name: FuzzTypeIdent::arbitrary(u)?,
            type_params: {
                let count: u8 = u.arbitrary()?;
                let count = count % 3;
                let mut params = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    params.push(FuzzTypeParam::arbitrary_with_depth(u, 0)?);
                }
                params
            },
            operations,
        })
    }
}

/// An effect operation declaration.
#[derive(Debug, Clone)]
pub struct FuzzOperation {
    pub name: FuzzIdent,
    pub params: Vec<FuzzParam>,
    pub return_type: FuzzType,
}

impl FuzzOperation {
    pub fn to_source(&self) -> String {
        let mut s = String::from("op ");
        s.push_str(&self.name.to_source());
        s.push('(');
        s.push_str(
            &self
                .params
                .iter()
                .map(|p| p.to_source())
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push_str(") -> ");
        s.push_str(&self.return_type.to_source());
        s.push(';');
        s
    }
}

impl<'a> Arbitrary<'a> for FuzzOperation {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let param_count: u8 = u.arbitrary()?;
        let param_count = param_count % 4;

        let mut params = Vec::with_capacity(param_count as usize);
        for _ in 0..param_count {
            params.push(FuzzParam::arbitrary_with_depth(u, 0)?);
        }

        Ok(FuzzOperation {
            name: FuzzIdent::arbitrary(u)?,
            params,
            return_type: FuzzType::arbitrary_with_depth(u, 0)?,
        })
    }
}

// ============================================================
// Handler Declaration
// ============================================================

/// A handler declaration.
#[derive(Debug, Clone)]
pub struct FuzzHandler {
    pub kind: FuzzHandlerKind,
    pub name: FuzzTypeIdent,
    pub type_params: Vec<FuzzTypeParam>,
    pub effect_type: FuzzType,
    pub return_clause: Option<FuzzReturnClause>,
    pub op_impls: Vec<FuzzOpImpl>,
}

impl FuzzHandler {
    pub fn to_source(&self) -> String {
        let mut s = String::new();

        s.push_str(&self.kind.to_source());
        s.push_str(" handler ");
        s.push_str(&self.name.to_source());

        if !self.type_params.is_empty() {
            s.push('<');
            s.push_str(
                &self
                    .type_params
                    .iter()
                    .map(|p| p.to_source())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            s.push('>');
        }

        s.push_str(" for ");
        s.push_str(&self.effect_type.to_source());
        s.push_str(" {\n");

        if let Some(ret) = &self.return_clause {
            s.push_str("    ");
            s.push_str(&ret.to_source());
            s.push('\n');
        }

        for op in &self.op_impls {
            s.push_str("    ");
            s.push_str(&op.to_source());
            s.push('\n');
        }

        s.push('}');
        s
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        let op_count: u8 = u.arbitrary()?;
        let op_count = (op_count % 3) + 1;

        let mut op_impls = Vec::with_capacity(op_count as usize);
        for _ in 0..op_count {
            op_impls.push(FuzzOpImpl::arbitrary_with_depth(u, depth)?);
        }

        let has_return: bool = u.arbitrary()?;

        Ok(FuzzHandler {
            kind: FuzzHandlerKind::arbitrary(u)?,
            name: FuzzTypeIdent::arbitrary(u)?,
            type_params: {
                let count: u8 = u.arbitrary()?;
                let count = count % 3;
                let mut params = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    params.push(FuzzTypeParam::arbitrary_with_depth(u, depth)?);
                }
                params
            },
            effect_type: FuzzType::arbitrary_with_depth(u, depth)?,
            return_clause: if has_return {
                Some(FuzzReturnClause::arbitrary_with_depth(u, depth)?)
            } else {
                None
            },
            op_impls,
        })
    }
}

impl<'a> Arbitrary<'a> for FuzzHandler {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Self::arbitrary_with_depth(u, 0)
    }
}

/// Handler kind: deep or shallow.
#[derive(Debug, Clone)]
pub enum FuzzHandlerKind {
    Deep,
    Shallow,
}

impl FuzzHandlerKind {
    pub fn to_source(&self) -> String {
        match self {
            FuzzHandlerKind::Deep => "deep".to_string(),
            FuzzHandlerKind::Shallow => "shallow".to_string(),
        }
    }
}

impl<'a> Arbitrary<'a> for FuzzHandlerKind {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(if u.arbitrary()? {
            FuzzHandlerKind::Deep
        } else {
            FuzzHandlerKind::Shallow
        })
    }
}

/// A handler return clause.
#[derive(Debug, Clone)]
pub struct FuzzReturnClause {
    pub param: FuzzIdent,
    pub body: FuzzBlock,
}

impl FuzzReturnClause {
    pub fn to_source(&self) -> String {
        format!("return({}) {}", self.param.to_source(), self.body.to_source())
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        Ok(FuzzReturnClause {
            param: FuzzIdent::arbitrary(u)?,
            body: FuzzBlock::arbitrary_with_depth(u, depth)?,
        })
    }
}

/// An operation implementation in a handler.
#[derive(Debug, Clone)]
pub struct FuzzOpImpl {
    pub name: FuzzIdent,
    pub params: Vec<FuzzIdent>,
    pub body: FuzzBlock,
}

impl FuzzOpImpl {
    pub fn to_source(&self) -> String {
        let mut s = String::from("op ");
        s.push_str(&self.name.to_source());
        s.push('(');
        s.push_str(
            &self
                .params
                .iter()
                .map(|p| p.to_source())
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push_str(") ");
        s.push_str(&self.body.to_source());
        s
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        let param_count: u8 = u.arbitrary()?;
        let param_count = param_count % 4;

        let mut params = Vec::with_capacity(param_count as usize);
        for _ in 0..param_count {
            params.push(FuzzIdent::arbitrary(u)?);
        }

        Ok(FuzzOpImpl {
            name: FuzzIdent::arbitrary(u)?,
            params,
            body: FuzzBlock::arbitrary_with_depth(u, depth)?,
        })
    }
}

// ============================================================
// Type Alias Declaration
// ============================================================

/// A type alias declaration.
#[derive(Debug, Clone)]
pub struct FuzzTypeAlias {
    pub visibility: FuzzVisibility,
    pub name: FuzzTypeIdent,
    pub type_params: Vec<FuzzTypeParam>,
    pub ty: FuzzType,
}

impl FuzzTypeAlias {
    pub fn to_source(&self) -> String {
        let mut s = String::new();

        s.push_str(&self.visibility.to_source());
        if !s.is_empty() {
            s.push(' ');
        }

        s.push_str("type ");
        s.push_str(&self.name.to_source());

        if !self.type_params.is_empty() {
            s.push('<');
            s.push_str(
                &self
                    .type_params
                    .iter()
                    .map(|p| p.to_source())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            s.push('>');
        }

        s.push_str(" = ");
        s.push_str(&self.ty.to_source());
        s.push(';');
        s
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        Ok(FuzzTypeAlias {
            visibility: FuzzVisibility::arbitrary(u)?,
            name: FuzzTypeIdent::arbitrary(u)?,
            type_params: {
                let count: u8 = u.arbitrary()?;
                let count = count % 3;
                let mut params = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    params.push(FuzzTypeParam::arbitrary_with_depth(u, depth)?);
                }
                params
            },
            ty: FuzzType::arbitrary_with_depth(u, depth)?,
        })
    }
}

// ============================================================
// Const Declaration
// ============================================================

/// A const declaration.
#[derive(Debug, Clone)]
pub struct FuzzConst {
    pub visibility: FuzzVisibility,
    pub name: FuzzIdent,
    pub ty: FuzzType,
    pub value: FuzzExpr,
}

impl FuzzConst {
    pub fn to_source(&self) -> String {
        let mut s = String::new();

        s.push_str(&self.visibility.to_source());
        if !s.is_empty() {
            s.push(' ');
        }

        s.push_str("const ");
        s.push_str(&self.name.to_source());
        s.push_str(": ");
        s.push_str(&self.ty.to_source());
        s.push_str(" = ");
        s.push_str(&self.value.to_source());
        s.push(';');
        s
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        Ok(FuzzConst {
            visibility: FuzzVisibility::arbitrary(u)?,
            name: FuzzIdent::arbitrary(u)?,
            ty: FuzzType::arbitrary_with_depth(u, depth)?,
            value: FuzzExpr::arbitrary_with_depth(u, depth)?,
        })
    }
}

// ============================================================
// Common Components
// ============================================================

/// Visibility modifier.
#[derive(Debug, Clone)]
pub enum FuzzVisibility {
    Private,
    Public,
    PublicCrate,
    PublicSuper,
}

impl FuzzVisibility {
    pub fn to_source(&self) -> String {
        match self {
            FuzzVisibility::Private => String::new(),
            FuzzVisibility::Public => "pub".to_string(),
            FuzzVisibility::PublicCrate => "pub(crate)".to_string(),
            FuzzVisibility::PublicSuper => "pub(super)".to_string(),
        }
    }
}

impl<'a> Arbitrary<'a> for FuzzVisibility {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let choice: u8 = u.arbitrary()?;
        Ok(match choice % 4 {
            0 => FuzzVisibility::Private,
            1 => FuzzVisibility::Public,
            2 => FuzzVisibility::PublicCrate,
            _ => FuzzVisibility::PublicSuper,
        })
    }
}

/// A type parameter.
#[derive(Debug, Clone)]
pub struct FuzzTypeParam {
    pub name: FuzzTypeIdent,
    pub bounds: Vec<FuzzType>,
}

impl FuzzTypeParam {
    pub fn to_source(&self) -> String {
        let mut s = self.name.to_source();
        if !self.bounds.is_empty() {
            s.push_str(": ");
            s.push_str(
                &self
                    .bounds
                    .iter()
                    .map(|b| b.to_source())
                    .collect::<Vec<_>>()
                    .join(" + "),
            );
        }
        s
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, _depth: u8) -> arbitrary::Result<Self> {
        let bound_count: u8 = u.arbitrary()?;
        let bound_count = bound_count % 3;

        let mut bounds = Vec::with_capacity(bound_count as usize);
        for _ in 0..bound_count {
            bounds.push(FuzzType::arbitrary_simple(u)?);
        }

        Ok(FuzzTypeParam {
            name: FuzzTypeIdent::arbitrary(u)?,
            bounds,
        })
    }
}

/// A function parameter.
#[derive(Debug, Clone)]
pub struct FuzzParam {
    pub name: FuzzIdent,
    pub ty: FuzzType,
}

impl FuzzParam {
    pub fn to_source(&self) -> String {
        format!("{}: {}", self.name.to_source(), self.ty.to_source())
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        Ok(FuzzParam {
            name: FuzzIdent::arbitrary(u)?,
            ty: FuzzType::arbitrary_with_depth(u, depth)?,
        })
    }
}

// ============================================================
// Types
// ============================================================

/// A type expression.
#[derive(Debug, Clone)]
pub enum FuzzType {
    /// Simple type path: `i32`, `String`, `Vec<T>`
    Path {
        name: FuzzTypeIdent,
        args: Vec<FuzzType>,
    },
    /// Reference: `&T`, `&mut T`
    Reference {
        mutable: bool,
        inner: Box<FuzzType>,
    },
    /// Array: `[T; N]`
    Array {
        element: Box<FuzzType>,
        size: u64,
    },
    /// Slice: `[T]`
    Slice { element: Box<FuzzType> },
    /// Tuple: `(T, U, V)`
    Tuple(Vec<FuzzType>),
    /// Unit type: `()`
    Unit,
    /// Never type: `!`
    Never,
    /// Inferred: `_`
    Inferred,
}

impl FuzzType {
    pub fn to_source(&self) -> String {
        match self {
            FuzzType::Path { name, args } => {
                let mut s = name.to_source();
                if !args.is_empty() {
                    s.push('<');
                    s.push_str(
                        &args
                            .iter()
                            .map(|a| a.to_source())
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                    s.push('>');
                }
                s
            }
            FuzzType::Reference { mutable, inner } => {
                if *mutable {
                    format!("&mut {}", inner.to_source())
                } else {
                    format!("&{}", inner.to_source())
                }
            }
            FuzzType::Array { element, size } => {
                format!("[{}; {}]", element.to_source(), size)
            }
            FuzzType::Slice { element } => {
                format!("[{}]", element.to_source())
            }
            FuzzType::Tuple(types) => {
                let mut s = String::from("(");
                s.push_str(
                    &types
                        .iter()
                        .map(|t| t.to_source())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                if types.len() == 1 {
                    s.push(','); // Single-element tuple needs trailing comma
                }
                s.push(')');
                s
            }
            FuzzType::Unit => "()".to_string(),
            FuzzType::Never => "!".to_string(),
            FuzzType::Inferred => "_".to_string(),
        }
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        if depth >= MAX_DEPTH {
            return Self::arbitrary_simple(u);
        }

        let choice: u8 = u.arbitrary()?;
        Ok(match choice % 8 {
            0 => {
                // Path with type args
                let arg_count: u8 = u.arbitrary()?;
                let arg_count = arg_count % 3;
                let mut args = Vec::with_capacity(arg_count as usize);
                for _ in 0..arg_count {
                    args.push(FuzzType::arbitrary_with_depth(u, depth + 1)?);
                }
                FuzzType::Path {
                    name: FuzzTypeIdent::arbitrary(u)?,
                    args,
                }
            }
            1 => FuzzType::Reference {
                mutable: u.arbitrary()?,
                inner: Box::new(FuzzType::arbitrary_with_depth(u, depth + 1)?),
            },
            2 => FuzzType::Array {
                element: Box::new(FuzzType::arbitrary_with_depth(u, depth + 1)?),
                size: {
                    let size: u8 = u.arbitrary()?;
                    (size % 32) as u64
                },
            },
            3 => FuzzType::Slice {
                element: Box::new(FuzzType::arbitrary_with_depth(u, depth + 1)?),
            },
            4 => {
                let count: u8 = u.arbitrary()?;
                let count = count % 4;
                let mut types = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    types.push(FuzzType::arbitrary_with_depth(u, depth + 1)?);
                }
                FuzzType::Tuple(types)
            }
            5 => FuzzType::Unit,
            6 => FuzzType::Never,
            _ => Self::arbitrary_simple(u)?,
        })
    }

    /// Generate a simple type (no recursion).
    fn arbitrary_simple(u: &mut Unstructured<'_>) -> arbitrary::Result<Self> {
        const BUILTINS: &[&str] = &[
            "i8", "i16", "i32", "i64", "i128",
            "u8", "u16", "u32", "u64", "u128",
            "f32", "f64", "bool", "char", "String",
        ];

        let choice: u8 = u.arbitrary()?;
        let name = BUILTINS[choice as usize % BUILTINS.len()];
        Ok(FuzzType::Path {
            name: FuzzTypeIdent(name.to_string()),
            args: Vec::new(),
        })
    }
}

impl<'a> Arbitrary<'a> for FuzzType {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Self::arbitrary_with_depth(u, 0)
    }
}

// ============================================================
// Effect Rows
// ============================================================

/// An effect row.
#[derive(Debug, Clone)]
pub enum FuzzEffectRow {
    /// Pure: `pure` or `{}`
    Pure,
    /// Specific effects: `{IO, State<T>}`
    Effects(Vec<FuzzType>),
}

impl FuzzEffectRow {
    pub fn to_source(&self) -> String {
        match self {
            FuzzEffectRow::Pure => "pure".to_string(),
            FuzzEffectRow::Effects(effects) => {
                let mut s = String::from("{");
                s.push_str(
                    &effects
                        .iter()
                        .map(|e| e.to_source())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                s.push('}');
                s
            }
        }
    }
}

impl<'a> Arbitrary<'a> for FuzzEffectRow {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let is_pure: bool = u.arbitrary()?;
        if is_pure {
            return Ok(FuzzEffectRow::Pure);
        }

        let count: u8 = u.arbitrary()?;
        let count = (count % 3) + 1;
        let mut effects = Vec::with_capacity(count as usize);
        for _ in 0..count {
            effects.push(FuzzType::arbitrary_simple(u)?);
        }
        Ok(FuzzEffectRow::Effects(effects))
    }
}

// ============================================================
// Expressions
// ============================================================

/// An expression.
#[derive(Debug, Clone)]
pub enum FuzzExpr {
    /// Literal: `42`, `"hello"`, `true`
    Literal(FuzzLiteral),
    /// Identifier: `x`, `foo`
    Ident(FuzzIdent),
    /// Binary operation: `a + b`
    Binary {
        left: Box<FuzzExpr>,
        op: FuzzBinOp,
        right: Box<FuzzExpr>,
    },
    /// Unary operation: `-x`, `!b`
    Unary {
        op: FuzzUnaryOp,
        expr: Box<FuzzExpr>,
    },
    /// Function call: `foo(a, b)`
    Call {
        callee: Box<FuzzExpr>,
        args: Vec<FuzzExpr>,
    },
    /// If expression: `if cond { then } else { else }`
    If {
        cond: Box<FuzzExpr>,
        then_block: FuzzBlock,
        else_block: Option<FuzzBlock>,
    },
    /// Block expression: `{ stmts; expr }`
    Block(FuzzBlock),
    /// Tuple: `(a, b, c)`
    Tuple(Vec<FuzzExpr>),
    /// Array: `[a, b, c]`
    Array(Vec<FuzzExpr>),
    /// Field access: `a.field`
    Field {
        expr: Box<FuzzExpr>,
        field: FuzzIdent,
    },
    /// Index: `a[i]`
    Index {
        expr: Box<FuzzExpr>,
        index: Box<FuzzExpr>,
    },
    /// Return: `return expr`
    Return(Option<Box<FuzzExpr>>),
    /// Break: `break expr`
    Break(Option<Box<FuzzExpr>>),
    /// Continue
    Continue,
}

impl FuzzExpr {
    pub fn to_source(&self) -> String {
        match self {
            FuzzExpr::Literal(lit) => lit.to_source(),
            FuzzExpr::Ident(id) => id.to_source(),
            FuzzExpr::Binary { left, op, right } => {
                format!("({} {} {})", left.to_source(), op.to_source(), right.to_source())
            }
            FuzzExpr::Unary { op, expr } => {
                format!("({}{})", op.to_source(), expr.to_source())
            }
            FuzzExpr::Call { callee, args } => {
                let mut s = callee.to_source();
                s.push('(');
                s.push_str(
                    &args
                        .iter()
                        .map(|a| a.to_source())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                s.push(')');
                s
            }
            FuzzExpr::If {
                cond,
                then_block,
                else_block,
            } => {
                let mut s = format!("if {} {}", cond.to_source(), then_block.to_source());
                if let Some(else_b) = else_block {
                    s.push_str(" else ");
                    s.push_str(&else_b.to_source());
                }
                s
            }
            FuzzExpr::Block(block) => block.to_source(),
            FuzzExpr::Tuple(exprs) => {
                let mut s = String::from("(");
                s.push_str(
                    &exprs
                        .iter()
                        .map(|e| e.to_source())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                if exprs.len() == 1 {
                    s.push(',');
                }
                s.push(')');
                s
            }
            FuzzExpr::Array(exprs) => {
                let mut s = String::from("[");
                s.push_str(
                    &exprs
                        .iter()
                        .map(|e| e.to_source())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                s.push(']');
                s
            }
            FuzzExpr::Field { expr, field } => {
                format!("{}.{}", expr.to_source(), field.to_source())
            }
            FuzzExpr::Index { expr, index } => {
                format!("{}[{}]", expr.to_source(), index.to_source())
            }
            FuzzExpr::Return(Some(e)) => format!("return {}", e.to_source()),
            FuzzExpr::Return(None) => "return".to_string(),
            FuzzExpr::Break(Some(e)) => format!("break {}", e.to_source()),
            FuzzExpr::Break(None) => "break".to_string(),
            FuzzExpr::Continue => "continue".to_string(),
        }
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        if depth >= MAX_DEPTH {
            return Self::arbitrary_simple(u);
        }

        let choice: u8 = u.arbitrary()?;
        Ok(match choice % 14 {
            0 | 1 | 2 => FuzzExpr::Literal(FuzzLiteral::arbitrary(u)?),
            3 | 4 => FuzzExpr::Ident(FuzzIdent::arbitrary(u)?),
            5 => FuzzExpr::Binary {
                left: Box::new(FuzzExpr::arbitrary_with_depth(u, depth + 1)?),
                op: FuzzBinOp::arbitrary(u)?,
                right: Box::new(FuzzExpr::arbitrary_with_depth(u, depth + 1)?),
            },
            6 => FuzzExpr::Unary {
                op: FuzzUnaryOp::arbitrary(u)?,
                expr: Box::new(FuzzExpr::arbitrary_with_depth(u, depth + 1)?),
            },
            7 => {
                let arg_count: u8 = u.arbitrary()?;
                let arg_count = arg_count % 4;
                let mut args = Vec::with_capacity(arg_count as usize);
                for _ in 0..arg_count {
                    args.push(FuzzExpr::arbitrary_with_depth(u, depth + 1)?);
                }
                FuzzExpr::Call {
                    callee: Box::new(FuzzExpr::Ident(FuzzIdent::arbitrary(u)?)),
                    args,
                }
            }
            8 => {
                let has_else: bool = u.arbitrary()?;
                FuzzExpr::If {
                    cond: Box::new(FuzzExpr::arbitrary_with_depth(u, depth + 1)?),
                    then_block: FuzzBlock::arbitrary_with_depth(u, depth + 1)?,
                    else_block: if has_else {
                        Some(FuzzBlock::arbitrary_with_depth(u, depth + 1)?)
                    } else {
                        None
                    },
                }
            }
            9 => FuzzExpr::Block(FuzzBlock::arbitrary_with_depth(u, depth + 1)?),
            10 => {
                let count: u8 = u.arbitrary()?;
                let count = (count % 4) + 1;
                let mut exprs = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    exprs.push(FuzzExpr::arbitrary_with_depth(u, depth + 1)?);
                }
                FuzzExpr::Tuple(exprs)
            }
            11 => {
                let count: u8 = u.arbitrary()?;
                let count = count % 5;
                let mut exprs = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    exprs.push(FuzzExpr::arbitrary_with_depth(u, depth + 1)?);
                }
                FuzzExpr::Array(exprs)
            }
            12 => FuzzExpr::Field {
                expr: Box::new(FuzzExpr::Ident(FuzzIdent::arbitrary(u)?)),
                field: FuzzIdent::arbitrary(u)?,
            },
            _ => Self::arbitrary_simple(u)?,
        })
    }

    fn arbitrary_simple(u: &mut Unstructured<'_>) -> arbitrary::Result<Self> {
        let choice: u8 = u.arbitrary()?;
        Ok(match choice % 3 {
            0 => FuzzExpr::Literal(FuzzLiteral::arbitrary(u)?),
            1 => FuzzExpr::Ident(FuzzIdent::arbitrary(u)?),
            _ => FuzzExpr::Literal(FuzzLiteral::Int(42)),
        })
    }
}

impl<'a> Arbitrary<'a> for FuzzExpr {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Self::arbitrary_with_depth(u, 0)
    }
}

// ============================================================
// Literals
// ============================================================

/// A literal value.
#[derive(Debug, Clone)]
pub enum FuzzLiteral {
    Int(i64),
    Float(f64),
    String(String),
    Char(char),
    Bool(bool),
}

impl FuzzLiteral {
    pub fn to_source(&self) -> String {
        match self {
            FuzzLiteral::Int(n) => n.to_string(),
            FuzzLiteral::Float(f) => {
                let s = f.to_string();
                if s.contains('.') {
                    s
                } else {
                    format!("{}.0", s)
                }
            }
            FuzzLiteral::String(s) => format!("{:?}", s),
            FuzzLiteral::Char(c) => format!("{:?}", c),
            FuzzLiteral::Bool(b) => b.to_string(),
        }
    }
}

impl<'a> Arbitrary<'a> for FuzzLiteral {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let choice: u8 = u.arbitrary()?;
        Ok(match choice % 5 {
            0 => {
                let n: i32 = u.arbitrary()?;
                FuzzLiteral::Int(n as i64)
            }
            1 => {
                let f: f32 = u.arbitrary()?;
                // Avoid NaN and Inf
                let f = if f.is_finite() { f } else { 0.0 };
                FuzzLiteral::Float(f as f64)
            }
            2 => {
                // Generate safe strings
                const STRINGS: &[&str] = &[
                    "hello", "world", "test", "", "foo bar",
                    "line1\nline2", "tab\there", "quote\"test",
                ];
                let idx: usize = u.arbitrary()?;
                FuzzLiteral::String(STRINGS[idx % STRINGS.len()].to_string())
            }
            3 => {
                const CHARS: &[char] = &['a', 'z', 'A', 'Z', '0', '9', '\n', '\t', ' ', '!'];
                let idx: usize = u.arbitrary()?;
                FuzzLiteral::Char(CHARS[idx % CHARS.len()])
            }
            _ => FuzzLiteral::Bool(u.arbitrary()?),
        })
    }
}

// ============================================================
// Operators
// ============================================================

/// Binary operators.
#[derive(Debug, Clone)]
pub enum FuzzBinOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    // Comparison
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    // Logical
    And,
    Or,
    // Bitwise
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl FuzzBinOp {
    pub fn to_source(&self) -> &'static str {
        match self {
            FuzzBinOp::Add => "+",
            FuzzBinOp::Sub => "-",
            FuzzBinOp::Mul => "*",
            FuzzBinOp::Div => "/",
            FuzzBinOp::Mod => "%",
            FuzzBinOp::Eq => "==",
            FuzzBinOp::Ne => "!=",
            FuzzBinOp::Lt => "<",
            FuzzBinOp::Le => "<=",
            FuzzBinOp::Gt => ">",
            FuzzBinOp::Ge => ">=",
            FuzzBinOp::And => "&&",
            FuzzBinOp::Or => "||",
            FuzzBinOp::BitAnd => "&",
            FuzzBinOp::BitOr => "|",
            FuzzBinOp::BitXor => "^",
            FuzzBinOp::Shl => "<<",
            FuzzBinOp::Shr => ">>",
        }
    }
}

impl<'a> Arbitrary<'a> for FuzzBinOp {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let choice: u8 = u.arbitrary()?;
        Ok(match choice % 18 {
            0 => FuzzBinOp::Add,
            1 => FuzzBinOp::Sub,
            2 => FuzzBinOp::Mul,
            3 => FuzzBinOp::Div,
            4 => FuzzBinOp::Mod,
            5 => FuzzBinOp::Eq,
            6 => FuzzBinOp::Ne,
            7 => FuzzBinOp::Lt,
            8 => FuzzBinOp::Le,
            9 => FuzzBinOp::Gt,
            10 => FuzzBinOp::Ge,
            11 => FuzzBinOp::And,
            12 => FuzzBinOp::Or,
            13 => FuzzBinOp::BitAnd,
            14 => FuzzBinOp::BitOr,
            15 => FuzzBinOp::BitXor,
            16 => FuzzBinOp::Shl,
            _ => FuzzBinOp::Shr,
        })
    }
}

/// Unary operators.
#[derive(Debug, Clone)]
pub enum FuzzUnaryOp {
    Neg,
    Not,
    Ref,
    RefMut,
    Deref,
}

impl FuzzUnaryOp {
    pub fn to_source(&self) -> &'static str {
        match self {
            FuzzUnaryOp::Neg => "-",
            FuzzUnaryOp::Not => "!",
            FuzzUnaryOp::Ref => "&",
            FuzzUnaryOp::RefMut => "&mut ",
            FuzzUnaryOp::Deref => "*",
        }
    }
}

impl<'a> Arbitrary<'a> for FuzzUnaryOp {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let choice: u8 = u.arbitrary()?;
        Ok(match choice % 5 {
            0 => FuzzUnaryOp::Neg,
            1 => FuzzUnaryOp::Not,
            2 => FuzzUnaryOp::Ref,
            3 => FuzzUnaryOp::RefMut,
            _ => FuzzUnaryOp::Deref,
        })
    }
}

// ============================================================
// Blocks and Statements
// ============================================================

/// A block expression.
#[derive(Debug, Clone)]
pub struct FuzzBlock {
    pub statements: Vec<FuzzStatement>,
    pub expr: Option<Box<FuzzExpr>>,
}

impl FuzzBlock {
    pub fn to_source(&self) -> String {
        let mut s = String::from("{\n");
        for stmt in &self.statements {
            s.push_str("    ");
            s.push_str(&stmt.to_source());
            s.push('\n');
        }
        if let Some(expr) = &self.expr {
            s.push_str("    ");
            s.push_str(&expr.to_source());
            s.push('\n');
        }
        s.push('}');
        s
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        let stmt_count: u8 = u.arbitrary()?;
        let stmt_count = stmt_count % 4;

        let mut statements = Vec::with_capacity(stmt_count as usize);
        for _ in 0..stmt_count {
            statements.push(FuzzStatement::arbitrary_with_depth(u, depth)?);
        }

        let has_expr: bool = u.arbitrary()?;
        let expr = if has_expr {
            Some(Box::new(FuzzExpr::arbitrary_with_depth(u, depth)?))
        } else {
            None
        };

        Ok(FuzzBlock { statements, expr })
    }
}

impl<'a> Arbitrary<'a> for FuzzBlock {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Self::arbitrary_with_depth(u, 0)
    }
}

/// A statement.
#[derive(Debug, Clone)]
pub enum FuzzStatement {
    /// Let binding: `let x: T = expr;`
    Let {
        name: FuzzIdent,
        ty: Option<FuzzType>,
        value: Option<FuzzExpr>,
    },
    /// Expression statement: `expr;`
    Expr(FuzzExpr),
}

impl FuzzStatement {
    pub fn to_source(&self) -> String {
        match self {
            FuzzStatement::Let { name, ty, value } => {
                let mut s = String::from("let ");
                s.push_str(&name.to_source());
                if let Some(t) = ty {
                    s.push_str(": ");
                    s.push_str(&t.to_source());
                }
                if let Some(v) = value {
                    s.push_str(" = ");
                    s.push_str(&v.to_source());
                }
                s.push(';');
                s
            }
            FuzzStatement::Expr(expr) => {
                let mut s = expr.to_source();
                s.push(';');
                s
            }
        }
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        let is_let: bool = u.arbitrary()?;
        if is_let {
            let has_ty: bool = u.arbitrary()?;
            let has_value: bool = u.arbitrary()?;

            Ok(FuzzStatement::Let {
                name: FuzzIdent::arbitrary(u)?,
                ty: if has_ty {
                    Some(FuzzType::arbitrary_with_depth(u, depth)?)
                } else {
                    None
                },
                value: if has_value {
                    Some(FuzzExpr::arbitrary_with_depth(u, depth)?)
                } else {
                    None
                },
            })
        } else {
            Ok(FuzzStatement::Expr(FuzzExpr::arbitrary_with_depth(
                u, depth,
            )?))
        }
    }
}

impl<'a> Arbitrary<'a> for FuzzStatement {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Self::arbitrary_with_depth(u, 0)
    }
}

// ============================================================
// Patterns (for match expressions)
// ============================================================

/// A pattern for match expressions.
#[derive(Debug, Clone)]
pub enum FuzzPattern {
    /// Wildcard: `_`
    Wildcard,
    /// Identifier binding: `x`, `mut x`
    Binding {
        mutable: bool,
        name: FuzzIdent,
        subpattern: Option<Box<FuzzPattern>>,
    },
    /// Literal: `42`, `true`, `"hello"`
    Literal(FuzzLiteral),
    /// Tuple: `(a, b, c)`
    Tuple(Vec<FuzzPattern>),
    /// Struct: `Point { x, y }`
    Struct {
        name: FuzzTypeIdent,
        fields: Vec<(FuzzIdent, FuzzPattern)>,
    },
    /// Variant: `Some(x)`, `None`
    Variant {
        name: FuzzTypeIdent,
        fields: Option<Vec<FuzzPattern>>,
    },
    /// Or pattern: `A | B`
    Or(Vec<FuzzPattern>),
    /// Range: `0..10`, `'a'..='z'`
    Range {
        start: Option<Box<FuzzLiteral>>,
        end: Option<Box<FuzzLiteral>>,
        inclusive: bool,
    },
    /// Rest: `..`
    Rest,
}

impl FuzzPattern {
    pub fn to_source(&self) -> String {
        match self {
            FuzzPattern::Wildcard => "_".to_string(),
            FuzzPattern::Binding { mutable, name, subpattern } => {
                let mut s = String::new();
                if *mutable {
                    s.push_str("mut ");
                }
                s.push_str(&name.to_source());
                if let Some(sub) = subpattern {
                    s.push_str(" @ ");
                    s.push_str(&sub.to_source());
                }
                s
            }
            FuzzPattern::Literal(lit) => lit.to_source(),
            FuzzPattern::Tuple(pats) => {
                let mut s = String::from("(");
                s.push_str(
                    &pats
                        .iter()
                        .map(|p| p.to_source())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                if pats.len() == 1 {
                    s.push(',');
                }
                s.push(')');
                s
            }
            FuzzPattern::Struct { name, fields } => {
                let mut s = name.to_source();
                s.push_str(" { ");
                s.push_str(
                    &fields
                        .iter()
                        .map(|(n, p)| {
                            let pat_src = p.to_source();
                            if n.0 == pat_src {
                                n.to_source()
                            } else {
                                format!("{}: {}", n.to_source(), pat_src)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                s.push_str(" }");
                s
            }
            FuzzPattern::Variant { name, fields } => {
                let mut s = name.to_source();
                if let Some(pats) = fields {
                    s.push('(');
                    s.push_str(
                        &pats
                            .iter()
                            .map(|p| p.to_source())
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                    s.push(')');
                }
                s
            }
            FuzzPattern::Or(pats) => pats
                .iter()
                .map(|p| p.to_source())
                .collect::<Vec<_>>()
                .join(" | "),
            FuzzPattern::Range { start, end, inclusive } => {
                let mut s = String::new();
                if let Some(st) = start {
                    s.push_str(&st.to_source());
                }
                s.push_str(if *inclusive { "..=" } else { ".." });
                if let Some(en) = end {
                    s.push_str(&en.to_source());
                }
                s
            }
            FuzzPattern::Rest => "..".to_string(),
        }
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        if depth >= MAX_DEPTH {
            return Self::arbitrary_simple(u);
        }

        let choice: u8 = u.arbitrary()?;
        Ok(match choice % 10 {
            0 | 1 => FuzzPattern::Wildcard,
            2 | 3 => {
                let has_sub: bool = u.arbitrary()?;
                FuzzPattern::Binding {
                    mutable: u.arbitrary()?,
                    name: FuzzIdent::arbitrary(u)?,
                    subpattern: if has_sub && depth < MAX_DEPTH - 1 {
                        Some(Box::new(FuzzPattern::arbitrary_with_depth(u, depth + 1)?))
                    } else {
                        None
                    },
                }
            }
            4 => FuzzPattern::Literal(FuzzLiteral::arbitrary(u)?),
            5 => {
                let count: u8 = u.arbitrary()?;
                let count = (count % 4) + 1;
                let mut pats = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    pats.push(FuzzPattern::arbitrary_with_depth(u, depth + 1)?);
                }
                FuzzPattern::Tuple(pats)
            }
            6 => {
                let field_count: u8 = u.arbitrary()?;
                let field_count = field_count % 4;
                let mut fields = Vec::with_capacity(field_count as usize);
                for _ in 0..field_count {
                    fields.push((
                        FuzzIdent::arbitrary(u)?,
                        FuzzPattern::arbitrary_with_depth(u, depth + 1)?,
                    ));
                }
                FuzzPattern::Struct {
                    name: FuzzTypeIdent::arbitrary(u)?,
                    fields,
                }
            }
            7 => {
                let has_fields: bool = u.arbitrary()?;
                FuzzPattern::Variant {
                    name: FuzzTypeIdent::arbitrary(u)?,
                    fields: if has_fields {
                        let count: u8 = u.arbitrary()?;
                        let count = (count % 3) + 1;
                        let mut pats = Vec::with_capacity(count as usize);
                        for _ in 0..count {
                            pats.push(FuzzPattern::arbitrary_with_depth(u, depth + 1)?);
                        }
                        Some(pats)
                    } else {
                        None
                    },
                }
            }
            8 => {
                let count: u8 = u.arbitrary()?;
                let count = (count % 3) + 2; // At least 2 alternatives
                let mut pats = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    pats.push(FuzzPattern::arbitrary_with_depth(u, depth + 1)?);
                }
                FuzzPattern::Or(pats)
            }
            _ => Self::arbitrary_simple(u)?,
        })
    }

    fn arbitrary_simple(u: &mut Unstructured<'_>) -> arbitrary::Result<Self> {
        let choice: u8 = u.arbitrary()?;
        Ok(match choice % 4 {
            0 => FuzzPattern::Wildcard,
            1 => FuzzPattern::Binding {
                mutable: false,
                name: FuzzIdent::arbitrary(u)?,
                subpattern: None,
            },
            2 => FuzzPattern::Literal(FuzzLiteral::arbitrary(u)?),
            _ => FuzzPattern::Wildcard,
        })
    }
}

impl<'a> Arbitrary<'a> for FuzzPattern {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Self::arbitrary_with_depth(u, 0)
    }
}

/// A match arm: `pattern => expr`
#[derive(Debug, Clone)]
pub struct FuzzMatchArm {
    pub pattern: FuzzPattern,
    pub guard: Option<FuzzExpr>,
    pub body: FuzzExpr,
}

impl FuzzMatchArm {
    pub fn to_source(&self) -> String {
        let mut s = self.pattern.to_source();
        if let Some(guard) = &self.guard {
            s.push_str(" if ");
            s.push_str(&guard.to_source());
        }
        s.push_str(" => ");
        s.push_str(&self.body.to_source());
        s
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        let has_guard: bool = u.arbitrary()?;
        Ok(FuzzMatchArm {
            pattern: FuzzPattern::arbitrary_with_depth(u, depth)?,
            guard: if has_guard && depth < MAX_DEPTH {
                Some(FuzzExpr::arbitrary_with_depth(u, depth + 1)?)
            } else {
                None
            },
            body: FuzzExpr::arbitrary_with_depth(u, depth)?,
        })
    }
}

impl<'a> Arbitrary<'a> for FuzzMatchArm {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Self::arbitrary_with_depth(u, 0)
    }
}

/// A match expression.
#[derive(Debug, Clone)]
pub struct FuzzMatch {
    pub scrutinee: FuzzExpr,
    pub arms: Vec<FuzzMatchArm>,
}

impl FuzzMatch {
    pub fn to_source(&self) -> String {
        let mut s = String::from("match ");
        s.push_str(&self.scrutinee.to_source());
        s.push_str(" {\n");
        for arm in &self.arms {
            s.push_str("    ");
            s.push_str(&arm.to_source());
            s.push_str(",\n");
        }
        s.push('}');
        s
    }

    fn arbitrary_with_depth(u: &mut Unstructured<'_>, depth: u8) -> arbitrary::Result<Self> {
        let arm_count: u8 = u.arbitrary()?;
        let arm_count = (arm_count % 4) + 1; // At least 1 arm

        let mut arms = Vec::with_capacity(arm_count as usize);
        for _ in 0..arm_count {
            arms.push(FuzzMatchArm::arbitrary_with_depth(u, depth)?);
        }

        Ok(FuzzMatch {
            scrutinee: FuzzExpr::arbitrary_with_depth(u, depth)?,
            arms,
        })
    }
}

impl<'a> Arbitrary<'a> for FuzzMatch {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Self::arbitrary_with_depth(u, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_program_generation() {
        let data = [42u8; 256];
        let mut u = Unstructured::new(&data);
        let program = FuzzProgram::arbitrary(&mut u).unwrap();
        let source = program.to_source();
        assert!(!source.is_empty());
        // Should be parseable
        println!("Generated:\n{}", source);
    }

    #[test]
    fn test_pattern_generation() {
        let data = [99u8; 128];
        let mut u = Unstructured::new(&data);
        let pattern = FuzzPattern::arbitrary(&mut u).unwrap();
        let source = pattern.to_source();
        assert!(!source.is_empty());
        println!("Pattern:\n{}", source);
    }

    #[test]
    fn test_match_generation() {
        let data = [55u8; 256];
        let mut u = Unstructured::new(&data);
        let match_expr = FuzzMatch::arbitrary(&mut u).unwrap();
        let source = match_expr.to_source();
        assert!(source.starts_with("match "));
        println!("Match:\n{}", source);
    }

    #[test]
    fn test_function_generation() {
        let data = [123u8; 128];
        let mut u = Unstructured::new(&data);
        let func = FuzzFunction::arbitrary(&mut u).unwrap();
        let source = func.to_source();
        assert!(source.starts_with("fn ") || source.starts_with("pub ") || source.starts_with("pub("));
        println!("Function:\n{}", source);
    }

    #[test]
    fn test_effect_generation() {
        let data = [77u8; 64];
        let mut u = Unstructured::new(&data);
        let effect = FuzzEffect::arbitrary(&mut u).unwrap();
        let source = effect.to_source();
        assert!(source.starts_with("effect "));
        println!("Effect:\n{}", source);
    }
}
