// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use lazy_static::lazy_static;
use logger::prelude::*;
use std::{env, path::PathBuf, process::Command};

const WORKSPACE_BUILD_ERROR_MSG: &str = r#"
    Unable to build all workspace binaries. Cannot continue running tests.

    Try running 'cargo build --all --bins' yourself.
"#;

lazy_static! {
    // Global flag indicating if all binaries in the workspace have been built.
    // 全局标志，指示是否已构建工作空间中的所有二进制文件。
    static ref WORKSPACE_BUILT: bool = {
        info!("Building project binaries");
        let args = if cfg!(debug_assertions) {
            vec!["build", "--all", "--bins"]
        } else {
            vec!["build", "--all", "--bins", "--release"]
        };


        let cargo_build = Command::new("cargo")
            .current_dir(workspace_root())
            .args(&args)
            .output()
            .expect(WORKSPACE_BUILD_ERROR_MSG);
        if !cargo_build.status.success() {
            panic!(WORKSPACE_BUILD_ERROR_MSG);
        }

        info!("Finished building project binaries");

        true
    };
}

// Path to top level workspace 工作空间顶层目录
pub fn workspace_root() -> PathBuf {
    let mut path = build_dir();
    while !path.ends_with("target") {
        path.pop();
    }
    path.pop();
    path
}

// Path to the directory where build artifacts live.
// 构建工件所在目录的路径
//TODO 也许添加一个环境变量指向编译的二进制
//TODO maybe add an Environment Variable which points to built binaries
pub fn build_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .map(|mut path| {
            path.pop();
            if path.ends_with("deps") {
                path.pop();
            }
            path
        })
        .expect("Can't find the build directory. Cannot continue running tests")
}

// Path to a specified binary
// 特定二进制的路径
pub fn get_bin<S: AsRef<str>>(bin_name: S) -> PathBuf {
    // We have to check to see if the workspace is built first to ensure that the binaries we're
    // testing are up to date.
    //我们必须首先检查工作区是否已构建，以确保我们正在测试的二进制文件是最新的。
    if !*WORKSPACE_BUILT {
        panic!(WORKSPACE_BUILD_ERROR_MSG);
    }

    let bin_name = bin_name.as_ref();
    let bin_path = build_dir().join(format!("{}{}", bin_name, env::consts::EXE_SUFFIX));

    // If the binary doesn't exist then either building them failed somehow or the supplied binary
    // name doesn't match any binaries this workspace can produce.
    // 如果二进制文件不存在，则以某种方式构建它们失败，或者提供的二进制名称与此工作空间可以生成的任何二进制文件都不匹配。
    if !bin_path.exists() {
        panic!(format!(
            "Can't find binary '{}' in expected path {:?}",
            bin_name, bin_path
        ));
    }

    bin_path
}
