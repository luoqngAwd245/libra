// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! This compiles all the `.proto` files under `src/` directory.
//! 这个模块编译 `src/` 下所有的`.proto`文件
//! For example, if there is a file `src/a/b/c.proto`, it will generate `src/a/b/c.rs` and
//! `src/a/b/c_grpc.rs`.
//! 比如，有一个名为 `src/a/b/c.proto`的文件， 执行main后会生成`src/a/b/c.rs` 和  `src/a/b/c_grpc.rs`

fn main() {
    let proto_root = "src";
    let dependent_root = "../../types/src/proto";
    let mempool_dependent_root = "../../mempool/src/proto/shared";

    build_helpers::build_helpers::compile_proto(
        proto_root,
        vec![dependent_root, mempool_dependent_root],
        true,
    );
}
