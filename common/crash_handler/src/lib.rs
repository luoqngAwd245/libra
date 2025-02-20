// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use backtrace::Backtrace;
use logger::prelude::*;
use serde::Serialize;
use std::{
    panic::{self, PanicInfo},
    process, thread, time,
};

#[derive(Debug, Serialize)]
pub struct CrashInfo {
    details: String,
    backtrace: String,
}

/// Invoke to ensure process exits on a thread panic.
///
/// Tokio's default behavior is to catch panics and ignore them.  Invoking this function will
/// ensure that all subsequent thread panics (even Tokio threads) will report the
/// details/backtrace and then exit.
/// 调用以确保进程在线程崩溃时退出。
///
/// Tokio的默认行为是捕获恐慌并忽略它们。 调用此功能将确保所有后续线程紧急情况（甚至是Tokio线程）
/// 将报告详细信息/回溯，然后退出。
pub fn setup_panic_handler() {
    panic::set_hook(Box::new(move |pi: &PanicInfo<'_>| {
        handle_panic(pi);
    }));
}

// Formats and logs panic information
// 格式化和记录恐慌信息
fn handle_panic(panic_info: &PanicInfo<'_>) {
    // The Display formatter for a PanicInfo contains the message, payload and location.
    // PanicInfo的显示格式化程序包含消息，有效负载和位置。
    let details = format!("{}", panic_info);
    let backtrace = format!("{:#?}", Backtrace::new());

    let info = CrashInfo { details, backtrace };
    crit!("{}", toml::to_string_pretty(&info).unwrap());

    // Provide some time to save the log to disk
    thread::sleep(time::Duration::from_millis(100));

    // Kill the process
    process::exit(12);
}
