//! SIMD Operations for Blood Runtime
//!
//! Platform-native SIMD intrinsics for high-performance computing.
//!
//! ## Design
//!
//! Blood provides two levels of SIMD support:
//! 1. **Auto-vectorization** - The compiler auto-vectorizes suitable loops
//! 2. **Explicit SIMD** - This module provides explicit SIMD types and operations
//!
//! ## Platform Support
//!
//! | Platform | ISA | Status |
//! |----------|-----|--------|
//! | x86_64 | SSE4.2, AVX2, AVX-512 | Supported |
//! | AArch64 | NEON, SVE | Planned |
//! | WASM | SIMD128 | Planned |
//!
//! ## Usage
//!
//! ```ignore
//! use blood_runtime::simd::{f32x4, i32x4};
//!
//! let a = f32x4::new(1.0, 2.0, 3.0, 4.0);
//! let b = f32x4::new(5.0, 6.0, 7.0, 8.0);
//! let c = a + b; // SIMD addition
//! ```

use std::fmt;
use std::ops::{Add, Sub, Mul, Div, BitAnd, BitOr, BitXor, Neg, Not};

// ============================================================================
// 128-bit SIMD Types (SSE/NEON/WASM SIMD128)
// ============================================================================

/// 4x 32-bit floating point SIMD vector.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
pub struct f32x4([f32; 4]);

impl f32x4 {
    /// Create a new vector from components.
    #[inline]
    pub const fn new(a: f32, b: f32, c: f32, d: f32) -> Self {
        Self([a, b, c, d])
    }

    /// Create a vector with all lanes set to the same value.
    #[inline]
    pub const fn splat(value: f32) -> Self {
        Self([value, value, value, value])
    }

    /// Create a vector of zeros.
    #[inline]
    pub const fn zero() -> Self {
        Self([0.0, 0.0, 0.0, 0.0])
    }

    /// Load from a slice (must have at least 4 elements).
    #[inline]
    pub fn load(slice: &[f32]) -> Self {
        assert!(slice.len() >= 4, "slice too short for f32x4::load");
        Self([slice[0], slice[1], slice[2], slice[3]])
    }

    /// Load from an aligned pointer.
    ///
    /// # Safety
    /// The pointer must be aligned to 16 bytes and point to at least 4 f32 values.
    #[inline]
    pub unsafe fn load_aligned(ptr: *const f32) -> Self {
        debug_assert!(ptr as usize % 16 == 0, "unaligned pointer for load_aligned");
        Self([*ptr, *ptr.add(1), *ptr.add(2), *ptr.add(3)])
    }

    /// Store to a slice (must have at least 4 elements).
    #[inline]
    pub fn store(&self, slice: &mut [f32]) {
        assert!(slice.len() >= 4, "slice too short for f32x4::store");
        slice[0] = self.0[0];
        slice[1] = self.0[1];
        slice[2] = self.0[2];
        slice[3] = self.0[3];
    }

    /// Store to an aligned pointer.
    ///
    /// # Safety
    /// The pointer must be aligned to 16 bytes and have space for at least 4 f32 values.
    #[inline]
    pub unsafe fn store_aligned(&self, ptr: *mut f32) {
        debug_assert!(ptr as usize % 16 == 0, "unaligned pointer for store_aligned");
        *ptr = self.0[0];
        *ptr.add(1) = self.0[1];
        *ptr.add(2) = self.0[2];
        *ptr.add(3) = self.0[3];
    }

    /// Get a lane value.
    #[inline]
    pub fn get(&self, index: usize) -> f32 {
        self.0[index]
    }

    /// Set a lane value.
    #[inline]
    pub fn set(&mut self, index: usize, value: f32) {
        self.0[index] = value;
    }

    /// Horizontal sum of all lanes.
    #[inline]
    pub fn sum(&self) -> f32 {
        self.0[0] + self.0[1] + self.0[2] + self.0[3]
    }

    /// Horizontal product of all lanes.
    #[inline]
    pub fn product(&self) -> f32 {
        self.0[0] * self.0[1] * self.0[2] * self.0[3]
    }

    /// Minimum of all lanes.
    #[inline]
    pub fn min_lane(&self) -> f32 {
        self.0.iter().cloned().fold(f32::INFINITY, f32::min)
    }

    /// Maximum of all lanes.
    #[inline]
    pub fn max_lane(&self) -> f32 {
        self.0.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
    }

    /// Lane-wise minimum.
    #[inline]
    pub fn min(self, other: Self) -> Self {
        Self([
            self.0[0].min(other.0[0]),
            self.0[1].min(other.0[1]),
            self.0[2].min(other.0[2]),
            self.0[3].min(other.0[3]),
        ])
    }

    /// Lane-wise maximum.
    #[inline]
    pub fn max(self, other: Self) -> Self {
        Self([
            self.0[0].max(other.0[0]),
            self.0[1].max(other.0[1]),
            self.0[2].max(other.0[2]),
            self.0[3].max(other.0[3]),
        ])
    }

    /// Lane-wise absolute value.
    #[inline]
    pub fn abs(self) -> Self {
        Self([
            self.0[0].abs(),
            self.0[1].abs(),
            self.0[2].abs(),
            self.0[3].abs(),
        ])
    }

    /// Lane-wise square root.
    #[inline]
    pub fn sqrt(self) -> Self {
        Self([
            self.0[0].sqrt(),
            self.0[1].sqrt(),
            self.0[2].sqrt(),
            self.0[3].sqrt(),
        ])
    }

    /// Lane-wise reciprocal (1/x).
    #[inline]
    pub fn recip(self) -> Self {
        Self([
            1.0 / self.0[0],
            1.0 / self.0[1],
            1.0 / self.0[2],
            1.0 / self.0[3],
        ])
    }

    /// Fused multiply-add: self * a + b
    #[inline]
    pub fn mul_add(self, a: Self, b: Self) -> Self {
        Self([
            self.0[0].mul_add(a.0[0], b.0[0]),
            self.0[1].mul_add(a.0[1], b.0[1]),
            self.0[2].mul_add(a.0[2], b.0[2]),
            self.0[3].mul_add(a.0[3], b.0[3]),
        ])
    }

    /// Dot product of two vectors.
    #[inline]
    pub fn dot(self, other: Self) -> f32 {
        (self * other).sum()
    }
}

impl Add for f32x4 {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self([
            self.0[0] + rhs.0[0],
            self.0[1] + rhs.0[1],
            self.0[2] + rhs.0[2],
            self.0[3] + rhs.0[3],
        ])
    }
}

impl Sub for f32x4 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self([
            self.0[0] - rhs.0[0],
            self.0[1] - rhs.0[1],
            self.0[2] - rhs.0[2],
            self.0[3] - rhs.0[3],
        ])
    }
}

impl Mul for f32x4 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        Self([
            self.0[0] * rhs.0[0],
            self.0[1] * rhs.0[1],
            self.0[2] * rhs.0[2],
            self.0[3] * rhs.0[3],
        ])
    }
}

impl Div for f32x4 {
    type Output = Self;

    #[inline]
    fn div(self, rhs: Self) -> Self::Output {
        Self([
            self.0[0] / rhs.0[0],
            self.0[1] / rhs.0[1],
            self.0[2] / rhs.0[2],
            self.0[3] / rhs.0[3],
        ])
    }
}

impl Neg for f32x4 {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self::Output {
        Self([-self.0[0], -self.0[1], -self.0[2], -self.0[3]])
    }
}

impl Default for f32x4 {
    fn default() -> Self {
        Self::zero()
    }
}

impl fmt::Debug for f32x4 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "f32x4({}, {}, {}, {})", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

// ============================================================================
// 4x 64-bit floating point SIMD vector (AVX)
// ============================================================================

/// 4x 64-bit floating point SIMD vector (256-bit, AVX).
#[derive(Clone, Copy)]
#[repr(C, align(32))]
pub struct f64x4([f64; 4]);

impl f64x4 {
    /// Create a new vector from components.
    #[inline]
    pub const fn new(a: f64, b: f64, c: f64, d: f64) -> Self {
        Self([a, b, c, d])
    }

    /// Create a vector with all lanes set to the same value.
    #[inline]
    pub const fn splat(value: f64) -> Self {
        Self([value, value, value, value])
    }

    /// Create a vector of zeros.
    #[inline]
    pub const fn zero() -> Self {
        Self([0.0, 0.0, 0.0, 0.0])
    }

    /// Load from a slice.
    #[inline]
    pub fn load(slice: &[f64]) -> Self {
        assert!(slice.len() >= 4, "slice too short for f64x4::load");
        Self([slice[0], slice[1], slice[2], slice[3]])
    }

    /// Store to a slice.
    #[inline]
    pub fn store(&self, slice: &mut [f64]) {
        assert!(slice.len() >= 4, "slice too short for f64x4::store");
        slice[0] = self.0[0];
        slice[1] = self.0[1];
        slice[2] = self.0[2];
        slice[3] = self.0[3];
    }

    /// Horizontal sum.
    #[inline]
    pub fn sum(&self) -> f64 {
        self.0[0] + self.0[1] + self.0[2] + self.0[3]
    }

    /// Dot product.
    #[inline]
    pub fn dot(self, other: Self) -> f64 {
        (self * other).sum()
    }

    /// Fused multiply-add.
    #[inline]
    pub fn mul_add(self, a: Self, b: Self) -> Self {
        Self([
            self.0[0].mul_add(a.0[0], b.0[0]),
            self.0[1].mul_add(a.0[1], b.0[1]),
            self.0[2].mul_add(a.0[2], b.0[2]),
            self.0[3].mul_add(a.0[3], b.0[3]),
        ])
    }
}

impl Add for f64x4 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self([
            self.0[0] + rhs.0[0],
            self.0[1] + rhs.0[1],
            self.0[2] + rhs.0[2],
            self.0[3] + rhs.0[3],
        ])
    }
}

impl Sub for f64x4 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self([
            self.0[0] - rhs.0[0],
            self.0[1] - rhs.0[1],
            self.0[2] - rhs.0[2],
            self.0[3] - rhs.0[3],
        ])
    }
}

impl Mul for f64x4 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        Self([
            self.0[0] * rhs.0[0],
            self.0[1] * rhs.0[1],
            self.0[2] * rhs.0[2],
            self.0[3] * rhs.0[3],
        ])
    }
}

impl Div for f64x4 {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self {
        Self([
            self.0[0] / rhs.0[0],
            self.0[1] / rhs.0[1],
            self.0[2] / rhs.0[2],
            self.0[3] / rhs.0[3],
        ])
    }
}

impl Default for f64x4 {
    fn default() -> Self {
        Self::zero()
    }
}

impl fmt::Debug for f64x4 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "f64x4({}, {}, {}, {})", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

// ============================================================================
// Integer SIMD Types
// ============================================================================

/// 4x 32-bit signed integer SIMD vector.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
pub struct i32x4([i32; 4]);

impl i32x4 {
    /// Create a new vector from components.
    #[inline]
    pub const fn new(a: i32, b: i32, c: i32, d: i32) -> Self {
        Self([a, b, c, d])
    }

    /// Create a vector with all lanes set to the same value.
    #[inline]
    pub const fn splat(value: i32) -> Self {
        Self([value, value, value, value])
    }

    /// Create a vector of zeros.
    #[inline]
    pub const fn zero() -> Self {
        Self([0, 0, 0, 0])
    }

    /// Load from a slice.
    #[inline]
    pub fn load(slice: &[i32]) -> Self {
        assert!(slice.len() >= 4, "slice too short for i32x4::load");
        Self([slice[0], slice[1], slice[2], slice[3]])
    }

    /// Store to a slice.
    #[inline]
    pub fn store(&self, slice: &mut [i32]) {
        assert!(slice.len() >= 4, "slice too short for i32x4::store");
        slice[0] = self.0[0];
        slice[1] = self.0[1];
        slice[2] = self.0[2];
        slice[3] = self.0[3];
    }

    /// Get a lane value.
    #[inline]
    pub fn get(&self, index: usize) -> i32 {
        self.0[index]
    }

    /// Horizontal sum.
    #[inline]
    pub fn sum(&self) -> i32 {
        self.0[0].wrapping_add(self.0[1]).wrapping_add(self.0[2]).wrapping_add(self.0[3])
    }

    /// Lane-wise minimum.
    #[inline]
    pub fn min(self, other: Self) -> Self {
        Self([
            self.0[0].min(other.0[0]),
            self.0[1].min(other.0[1]),
            self.0[2].min(other.0[2]),
            self.0[3].min(other.0[3]),
        ])
    }

    /// Lane-wise maximum.
    #[inline]
    pub fn max(self, other: Self) -> Self {
        Self([
            self.0[0].max(other.0[0]),
            self.0[1].max(other.0[1]),
            self.0[2].max(other.0[2]),
            self.0[3].max(other.0[3]),
        ])
    }

    /// Lane-wise absolute value.
    #[inline]
    pub fn abs(self) -> Self {
        Self([
            self.0[0].abs(),
            self.0[1].abs(),
            self.0[2].abs(),
            self.0[3].abs(),
        ])
    }

    /// Left shift all lanes.
    #[inline]
    #[allow(clippy::should_implement_trait)] // SIMD type cannot implement std::ops::Shl generically
    pub fn shl(self, amount: u32) -> Self {
        Self([
            self.0[0] << amount,
            self.0[1] << amount,
            self.0[2] << amount,
            self.0[3] << amount,
        ])
    }

    /// Right shift all lanes (arithmetic).
    #[inline]
    #[allow(clippy::should_implement_trait)] // SIMD type cannot implement std::ops::Shr generically
    pub fn shr(self, amount: u32) -> Self {
        Self([
            self.0[0] >> amount,
            self.0[1] >> amount,
            self.0[2] >> amount,
            self.0[3] >> amount,
        ])
    }
}

impl Add for i32x4 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self([
            self.0[0].wrapping_add(rhs.0[0]),
            self.0[1].wrapping_add(rhs.0[1]),
            self.0[2].wrapping_add(rhs.0[2]),
            self.0[3].wrapping_add(rhs.0[3]),
        ])
    }
}

impl Sub for i32x4 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self([
            self.0[0].wrapping_sub(rhs.0[0]),
            self.0[1].wrapping_sub(rhs.0[1]),
            self.0[2].wrapping_sub(rhs.0[2]),
            self.0[3].wrapping_sub(rhs.0[3]),
        ])
    }
}

impl Mul for i32x4 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        Self([
            self.0[0].wrapping_mul(rhs.0[0]),
            self.0[1].wrapping_mul(rhs.0[1]),
            self.0[2].wrapping_mul(rhs.0[2]),
            self.0[3].wrapping_mul(rhs.0[3]),
        ])
    }
}

impl BitAnd for i32x4 {
    type Output = Self;
    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        Self([
            self.0[0] & rhs.0[0],
            self.0[1] & rhs.0[1],
            self.0[2] & rhs.0[2],
            self.0[3] & rhs.0[3],
        ])
    }
}

impl BitOr for i32x4 {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self([
            self.0[0] | rhs.0[0],
            self.0[1] | rhs.0[1],
            self.0[2] | rhs.0[2],
            self.0[3] | rhs.0[3],
        ])
    }
}

impl BitXor for i32x4 {
    type Output = Self;
    #[inline]
    fn bitxor(self, rhs: Self) -> Self {
        Self([
            self.0[0] ^ rhs.0[0],
            self.0[1] ^ rhs.0[1],
            self.0[2] ^ rhs.0[2],
            self.0[3] ^ rhs.0[3],
        ])
    }
}

impl Not for i32x4 {
    type Output = Self;
    #[inline]
    fn not(self) -> Self {
        Self([!self.0[0], !self.0[1], !self.0[2], !self.0[3]])
    }
}

impl Neg for i32x4 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self([
            self.0[0].wrapping_neg(),
            self.0[1].wrapping_neg(),
            self.0[2].wrapping_neg(),
            self.0[3].wrapping_neg(),
        ])
    }
}

impl Default for i32x4 {
    fn default() -> Self {
        Self::zero()
    }
}

impl fmt::Debug for i32x4 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "i32x4({}, {}, {}, {})", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

// ============================================================================
// 8x 32-bit integer vector (AVX2)
// ============================================================================

/// 8x 32-bit signed integer SIMD vector (256-bit, AVX2).
#[derive(Clone, Copy)]
#[repr(C, align(32))]
pub struct i32x8([i32; 8]);

impl i32x8 {
    /// Create from components.
    #[inline]
    #[allow(clippy::too_many_arguments)] // SIMD lane initialization requires one argument per lane
    pub const fn new(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32) -> Self {
        Self([a, b, c, d, e, f, g, h])
    }

    /// Splat a value to all lanes.
    #[inline]
    pub const fn splat(value: i32) -> Self {
        Self([value; 8])
    }

    /// Vector of zeros.
    #[inline]
    pub const fn zero() -> Self {
        Self([0; 8])
    }

    /// Load from slice.
    #[inline]
    pub fn load(slice: &[i32]) -> Self {
        assert!(slice.len() >= 8, "slice too short for i32x8::load");
        Self([
            slice[0], slice[1], slice[2], slice[3],
            slice[4], slice[5], slice[6], slice[7],
        ])
    }

    /// Store to slice.
    #[inline]
    pub fn store(&self, slice: &mut [i32]) {
        assert!(slice.len() >= 8, "slice too short for i32x8::store");
        slice[..8].copy_from_slice(&self.0);
    }

    /// Horizontal sum.
    #[inline]
    pub fn sum(&self) -> i32 {
        self.0.iter().fold(0i32, |acc, &x| acc.wrapping_add(x))
    }
}

impl Add for i32x8 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self([
            self.0[0].wrapping_add(rhs.0[0]),
            self.0[1].wrapping_add(rhs.0[1]),
            self.0[2].wrapping_add(rhs.0[2]),
            self.0[3].wrapping_add(rhs.0[3]),
            self.0[4].wrapping_add(rhs.0[4]),
            self.0[5].wrapping_add(rhs.0[5]),
            self.0[6].wrapping_add(rhs.0[6]),
            self.0[7].wrapping_add(rhs.0[7]),
        ])
    }
}

impl Default for i32x8 {
    fn default() -> Self {
        Self::zero()
    }
}

impl fmt::Debug for i32x8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f, "i32x8({}, {}, {}, {}, {}, {}, {}, {})",
            self.0[0], self.0[1], self.0[2], self.0[3],
            self.0[4], self.0[5], self.0[6], self.0[7]
        )
    }
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Check if AVX2 is available at runtime.
#[inline]
pub fn has_avx2() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        is_x86_feature_detected!("avx2")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// Check if AVX-512 is available at runtime.
#[inline]
pub fn has_avx512() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        is_x86_feature_detected!("avx512f")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// Check if NEON is available (always true on AArch64).
#[inline]
pub fn has_neon() -> bool {
    #[cfg(target_arch = "aarch64")]
    {
        true
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        false
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_f32x4_basic() {
        let a = f32x4::new(1.0, 2.0, 3.0, 4.0);
        let b = f32x4::new(5.0, 6.0, 7.0, 8.0);

        let sum = a + b;
        assert_eq!(sum.get(0), 6.0);
        assert_eq!(sum.get(1), 8.0);
        assert_eq!(sum.get(2), 10.0);
        assert_eq!(sum.get(3), 12.0);
    }

    #[test]
    fn test_f32x4_dot() {
        let a = f32x4::new(1.0, 2.0, 3.0, 4.0);
        let b = f32x4::new(5.0, 6.0, 7.0, 8.0);

        // 1*5 + 2*6 + 3*7 + 4*8 = 5 + 12 + 21 + 32 = 70
        assert_eq!(a.dot(b), 70.0);
    }

    #[test]
    fn test_f32x4_mul_add() {
        let a = f32x4::splat(2.0);
        let b = f32x4::splat(3.0);
        let c = f32x4::splat(1.0);

        // 2 * 3 + 1 = 7
        let result = a.mul_add(b, c);
        assert_eq!(result.get(0), 7.0);
    }

    #[test]
    fn test_i32x4_basic() {
        let a = i32x4::new(1, 2, 3, 4);
        let b = i32x4::new(10, 20, 30, 40);

        let sum = a + b;
        assert_eq!(sum.get(0), 11);
        assert_eq!(sum.get(1), 22);
        assert_eq!(sum.get(2), 33);
        assert_eq!(sum.get(3), 44);
    }

    #[test]
    fn test_i32x4_bitwise() {
        let a = i32x4::new(0b1010, 0b1100, 0b1111, 0b0000);
        let b = i32x4::new(0b1100, 0b1010, 0b1111, 0b0000);

        let and = a & b;
        assert_eq!(and.get(0), 0b1000);
        assert_eq!(and.get(1), 0b1000);
        assert_eq!(and.get(2), 0b1111);
        assert_eq!(and.get(3), 0b0000);
    }

    #[test]
    fn test_i32x4_shift() {
        let a = i32x4::new(1, 2, 4, 8);
        let shifted = a.shl(2);
        assert_eq!(shifted.get(0), 4);
        assert_eq!(shifted.get(1), 8);
        assert_eq!(shifted.get(2), 16);
        assert_eq!(shifted.get(3), 32);
    }

    #[test]
    fn test_load_store() {
        let data = [1.0f32, 2.0, 3.0, 4.0];
        let v = f32x4::load(&data);

        let mut output = [0.0f32; 4];
        v.store(&mut output);

        assert_eq!(data, output);
    }

    #[test]
    fn test_horizontal_ops() {
        let v = f32x4::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(v.sum(), 10.0);
        assert_eq!(v.min_lane(), 1.0);
        assert_eq!(v.max_lane(), 4.0);
    }
}
