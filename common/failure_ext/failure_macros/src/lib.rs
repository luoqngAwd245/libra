// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! Collection of convenience macros for error handling
//! 便利宏的集合，用于错误处理

/// Exits a function early with an `Error`.
///
/// Equivalent to the `bail!` macro, except a error type is provided instead of
/// a message.
/// 早期以“错误”退出功能。
///
///与`bail！`宏等效，除了提供错误类型而不是消息。
#[macro_export]
macro_rules! bail_err {
    ($e:expr) => {
        return Err(From::from($e));
    };
}
