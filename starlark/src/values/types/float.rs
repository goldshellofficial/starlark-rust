/*
 * Copyright 2018 The Starlark in Rust Authors.
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     https://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The floating point number type (3.14, 4e2).

use std::{
    cmp::Ordering,
    fmt::{self, Display, Write},
};

use gazebo::{any::AnyLifetime, prelude::*};

use crate::values::{
    num::Num, AllocFrozenValue, AllocValue, FrozenHeap, FrozenValue, Heap, SimpleValue,
    StarlarkValue, Value, ValueError,
};

const WRITE_PRECISION: usize = 6;

fn write_non_finite<W: fmt::Write>(output: &mut W, f: f64) -> fmt::Result {
    debug_assert!(f.is_nan() || f.is_infinite());
    if f.is_nan() {
        write!(output, "nan")
    } else {
        write!(
            output,
            "{}inf",
            if f.is_sign_positive() { "+" } else { "-" }
        )
    }
}

pub fn write_decimal<W: fmt::Write>(output: &mut W, f: f64) -> fmt::Result {
    if !f.is_finite() {
        write_non_finite(output, f)
    } else {
        write!(output, "{:.prec$}", f, prec = WRITE_PRECISION)
    }
}

pub fn write_scientific<W: fmt::Write>(
    output: &mut W,
    f: f64,
    exponent_char: char,
    strip_trailing_zeros: bool,
) -> fmt::Result {
    if !f.is_finite() {
        write_non_finite(output, f)
    } else {
        let abs = f.abs();
        let exponent = if f == 0.0 {
            0
        } else {
            abs.log10().floor() as i32
        };
        let normal = if f == 0.0 {
            0.0
        } else {
            abs / 10f64.powf(exponent as f64)
        };

        // start with "-" for a negative number
        if f.is_sign_negative() {
            output.write_char('-')?
        }

        // use the whole integral part of normal (a single digit)
        output.write_fmt(format_args!("{}", normal.trunc()))?;

        // calculate the fractional tail for given precision
        let mut tail = (normal.fract() * 10f64.powf(WRITE_PRECISION as f64)).round() as u64;
        let mut rev_tail = [0u8; WRITE_PRECISION];
        let mut rev_tail_len = 0;
        let mut removing_trailing_zeros = strip_trailing_zeros;
        for _ in 0..WRITE_PRECISION {
            let tail_digit = tail % 10;
            if tail_digit != 0 || !removing_trailing_zeros {
                removing_trailing_zeros = false;
                rev_tail[rev_tail_len] = tail_digit as u8;
                rev_tail_len += 1;
            }
            tail /= 10;
        }

        // write fractional part
        if rev_tail_len != 0 {
            output.write_char('.')?;
        }
        for digit in rev_tail[0..rev_tail_len].iter().rev() {
            output.write_char((b'0' + digit) as char)?;
        }

        // add exponent part
        output.write_char(exponent_char)?;
        output.write_fmt(format_args!("{:+03}", exponent))
    }
}

pub fn write_compact<W: fmt::Write>(output: &mut W, f: f64, exponent_char: char) -> fmt::Result {
    if !f.is_finite() {
        write_non_finite(output, f)
    } else {
        let abs = f.abs();
        let exponent = if f == 0.0 {
            0
        } else {
            abs.log10().floor() as i32
        };

        if exponent.abs() >= WRITE_PRECISION as i32 {
            // use scientific notation if exponent is outside of our precision (but strip 0s)
            write_scientific(output, f, exponent_char, true)
        } else if f.fract() == 0.0 {
            // make sure there's a fractional part even if the number doesn't have it
            output.write_fmt(format_args!("{:.1}", f))
        } else {
            // rely on the built-in formatting otherwise
            output.write_fmt(format_args!("{}", f))
        }
    }
}

#[derive(Clone, Dupe, Copy, Debug, AnyLifetime)]
pub struct StarlarkFloat(pub f64);

impl StarlarkFloat {
    /// The result of calling `type()` on floats.
    pub const TYPE: &'static str = "float";
}

impl<'v> AllocValue<'v> for f64 {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_simple(StarlarkFloat(self))
    }
}

impl AllocFrozenValue for f64 {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc_simple(StarlarkFloat(self))
    }
}

impl SimpleValue for StarlarkFloat {}

fn f64_arith_bin_op<'v, F>(
    left: f64,
    right: Value,
    heap: &'v Heap,
    op: &'static str,
    f: F,
) -> anyhow::Result<Value<'v>>
where
    F: FnOnce(f64, f64) -> anyhow::Result<f64>,
{
    if let Some(right) = right.unpack_num().map(|n| n.as_float()) {
        Ok(heap.alloc_simple(StarlarkFloat(f(left, right)?)))
    } else {
        ValueError::unsupported_with(&StarlarkFloat(left), op, right)
    }
}

impl Display for StarlarkFloat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_compact(f, self.0, 'e')
    }
}

impl<'v> StarlarkValue<'v> for StarlarkFloat {
    starlark_type!(StarlarkFloat::TYPE);

    fn equals(&self, other: Value) -> anyhow::Result<bool> {
        if other.unpack_num().is_some() {
            Ok(self.compare(other)? == Ordering::Equal)
        } else {
            Ok(false)
        }
    }

    fn collect_repr(&self, s: &mut String) {
        write!(s, "{}", self).unwrap()
    }

    fn to_json(&self) -> anyhow::Result<String> {
        // NaN/Infinity are not part of the JSON spec,
        // but it's unclear what should go here.
        // Perhaps strings with these values? null?
        // Leave it with these values for now.
        Ok(if self.0.is_nan() {
            "NaN".to_owned()
        } else if self.0.is_infinite() {
            if self.0.is_sign_positive() {
                "Infinity"
            } else {
                "-Infinity"
            }
            .to_owned()
        } else {
            self.to_string()
        })
    }

    fn to_bool(&self) -> bool {
        self.0 != 0.0
    }

    fn get_hash(&self) -> anyhow::Result<u64> {
        Ok(Num::from(self.0).get_hash())
    }

    fn plus(&self, heap: &'v Heap) -> anyhow::Result<Value<'v>> {
        Ok(heap.alloc_simple(*self))
    }

    fn minus(&self, heap: &'v Heap) -> anyhow::Result<Value<'v>> {
        Ok(heap.alloc_simple(StarlarkFloat(-self.0)))
    }

    fn add(&self, other: Value, heap: &'v Heap) -> anyhow::Result<Value<'v>> {
        f64_arith_bin_op(self.0, other, heap, "+", |l, r| Ok(l + r))
    }

    fn sub(&self, other: Value, heap: &'v Heap) -> anyhow::Result<Value<'v>> {
        f64_arith_bin_op(self.0, other, heap, "-", |l, r| Ok(l - r))
    }

    fn mul(&self, other: Value<'v>, heap: &'v Heap) -> anyhow::Result<Value<'v>> {
        f64_arith_bin_op(self.0, other, heap, "*", |l, r| Ok(l * r))
    }

    fn div(&self, other: Value, heap: &'v Heap) -> anyhow::Result<Value<'v>> {
        f64_arith_bin_op(self.0, other, heap, "/", |l, r| {
            if r == 0.0 {
                Err(ValueError::DivisionByZero.into())
            } else {
                Ok(l / r)
            }
        })
    }

    fn percent(&self, other: Value, heap: &'v Heap) -> anyhow::Result<Value<'v>> {
        f64_arith_bin_op(self.0, other, heap, "%", |a, b| {
            if b == 0.0 {
                Err(ValueError::DivisionByZero.into())
            } else {
                let r = a % b;
                if r == 0.0 {
                    Ok(0.0)
                } else {
                    Ok(if b.signum() != r.signum() { r + b } else { r })
                }
            }
        })
    }

    fn floor_div(&self, other: Value, heap: &'v Heap) -> anyhow::Result<Value<'v>> {
        f64_arith_bin_op(self.0, other, heap, "//", |l, r| {
            if r == 0.0 {
                Err(ValueError::DivisionByZero.into())
            } else {
                Ok((l / r).floor())
            }
        })
    }

    fn compare(&self, other: Value) -> anyhow::Result<Ordering> {
        if let Some(other_float) = other.unpack_num().map(|n| n.as_float()) {
            // According to the spec (https://github.com/bazelbuild/starlark/blob/689f54426951638ef5b7c41a14d8fc48e65c5f77/spec.md#floating-point-numbers)
            // All NaN values compare equal to each other, but greater than any non-NaN float value.
            match (self.0.is_nan(), other_float.is_nan()) {
                (true, true) => Ok(Ordering::Equal),
                (true, false) => Ok(Ordering::Greater),
                (false, true) => Ok(Ordering::Less),
                (false, false) => {
                    if let Some(ordering) = self.0.partial_cmp(&other_float) {
                        Ok(ordering)
                    } else {
                        // This shouldn't happen as we handle potential NaNs above
                        ValueError::unsupported_with(self, "==", other)
                    }
                }
            }
        } else {
            ValueError::unsupported_with(self, "==", other)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assert;

    fn non_finite(f: f64) -> String {
        let mut buf = String::new();
        write_non_finite(&mut buf, f).unwrap();
        buf
    }

    #[test]
    fn test_write_non_finite() {
        assert_eq!(non_finite(f64::NAN), "nan");
        assert_eq!(non_finite(f64::INFINITY), "+inf");
        assert_eq!(non_finite(f64::NEG_INFINITY), "-inf");
    }

    #[test]
    #[should_panic]
    fn test_write_non_finite_only_for_non_finite() {
        non_finite(0f64);
    }

    fn decimal(f: f64) -> String {
        let mut buf = String::new();
        write_decimal(&mut buf, f).unwrap();
        buf
    }

    #[test]
    fn test_write_decimal() {
        assert_eq!(decimal(f64::NAN), "nan");
        assert_eq!(decimal(f64::INFINITY), "+inf");
        assert_eq!(decimal(f64::NEG_INFINITY), "-inf");

        assert_eq!(decimal(0f64), "0.000000");
        assert_eq!(decimal(std::f64::consts::PI), "3.141593");
        assert_eq!(decimal(-std::f64::consts::E), "-2.718282");
        assert_eq!(decimal(1e10), "10000000000.000000");
    }

    fn scientific(f: f64) -> String {
        let mut buf = String::new();
        write_scientific(&mut buf, f, 'e', false).unwrap();
        buf
    }

    #[test]
    fn test_write_scientific() {
        assert_eq!(scientific(f64::NAN), "nan");
        assert_eq!(scientific(f64::INFINITY), "+inf");
        assert_eq!(scientific(f64::NEG_INFINITY), "-inf");

        assert_eq!(scientific(0f64), "0.000000e+00");
        assert_eq!(scientific(-0f64), "-0.000000e+00");
        assert_eq!(scientific(1.23e45), "1.230000e+45");
        assert_eq!(scientific(-3.14e-145), "-3.140000e-145");
        assert_eq!(scientific(1e300), "1.000000e+300");
    }

    fn compact(f: f64) -> String {
        let mut buf = String::new();
        write_compact(&mut buf, f, 'e').unwrap();
        buf
    }

    #[test]
    fn test_write_compact() {
        assert_eq!(compact(f64::NAN), "nan");
        assert_eq!(compact(f64::INFINITY), "+inf");
        assert_eq!(compact(f64::NEG_INFINITY), "-inf");

        assert_eq!(compact(0f64), "0.0");
        assert_eq!(compact(std::f64::consts::PI), "3.141592653589793");
        assert_eq!(compact(-std::f64::consts::E), "-2.718281828459045");
        assert_eq!(compact(1e10), "1e+10");
        assert_eq!(compact(1.23e45), "1.23e+45");
        assert_eq!(compact(-3.14e-145), "-3.14e-145");
        assert_eq!(compact(1e300), "1e+300");
    }

    #[test]
    fn test_arithmetic_operators() {
        assert::all_true(
            r#"
+1.0 == 1.0
-1.0 == 0. - 1.
1.0 + 2.0 == 3.0
1.0 - 2.0 == -1.0
2.0 * 3.0 == 6.0
5.0 / 2.0 == 2.5
5.0 % 3.0 == 2.0
5.0 // 2.0 == 2.0
"#,
        );
    }

    #[test]
    fn test_dictionary_key() {
        assert::pass(
            r#"
x = {0: 123}
assert_eq(x[0], 123)
assert_eq(x[0.0], 123)
assert_eq(x[-0.0], 123)
assert_eq(1 in x, False)
        "#,
        );
    }

    #[test]
    fn test_comparisons() {
        assert::all_true(
            r#"
+0.0 == -0.0
0.0 == 0
0 == 0.0
0 < 1.0
0.0 < 1
1 > 0.0
1.0 > 0
0.0 < float("nan")
float("+inf") < float("nan")
"#,
        );
    }

    #[test]
    fn test_comparisons_by_sorting() {
        assert::all_true(
            r#"
sorted([float('inf'), float('-inf'), float('nan'), 1e300, -1e300, 1.0, -1.0, 1, -1, 1e-300, -1e-300, 0, 0.0, float('-0.0'), 1e-300, -1e-300]) == [float('-inf'), -1e+300, -1.0, -1, -1e-300, -1e-300, 0, 0.0, -0.0, 1e-300, 1e-300, 1.0, 1, 1e+300, float('+inf'), float('nan')]
"#,
        );
    }
}
