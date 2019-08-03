// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use executable_helpers::helpers::{
    setup_executable, ARG_CONFIG_PATH, ARG_DISABLE_LOGGING, ARG_PEER_ID,
};
use signal_hook;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

fn register_signals(term: Arc<AtomicBool>) {
    for signal in &[
        signal_hook::SIGTERM,
        signal_hook::SIGINT,
        signal_hook::SIGHUP,
    ] {
        let term_clone = Arc::clone(&term);
        let thread = std::thread::current();
        unsafe {
            signal_hook::register(*signal, move || {
                term_clone.store(true, Ordering::Release);
                thread.unpark();
            })
            .expect("failed to register signal handler");
        }
    }
}
/// 节点启动入口函数
fn main() {
    // 执行可执行文件的常见设置
    let (mut config, _logger, _args) = setup_executable(
        "Libra single node".to_string(),
        vec![ARG_PEER_ID, ARG_CONFIG_PATH, ARG_DISABLE_LOGGING],
    );
    //设置环境，启动服务
    let (_ac_handle, _node_handle) = libra_node::main_node::setup_environment(&mut config);

    let term = Arc::new(AtomicBool::new(false));
     //注册新号钩子
    register_signals(Arc::clone(&term));
    
    while !term.load(Ordering::Acquire) {
        std::thread::park();
    }
}
