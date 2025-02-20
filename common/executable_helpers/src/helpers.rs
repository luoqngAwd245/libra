// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use clap::{value_t, App, Arg, ArgMatches};
use config::config::{NodeConfig, NodeConfigHelpers};
use logger::prelude::*;
use slog_scope::GlobalLoggerGuard;

// General args 常规参数
pub const ARG_PEER_ID: &str = "--peer_id";
pub const ARG_DISABLE_LOGGING: &str = "--no_logging";
pub const ARG_CONFIG_PATH: &str = "--config_path";

// Used for consensus 用于一致性
pub const ARG_NUM_PAYLOAD: &str = "--num_payload";
pub const ARG_PAYLOAD_SIZE: &str = "--payload_size";

pub fn load_configs_from_args(args: &ArgMatches<'_>) -> NodeConfig {
    let node_config;

    if args.is_present(ARG_CONFIG_PATH) {
        // Allow peer id over-ride via command line
        // 通过命令行允许对等ID覆盖
        let peer_id = value_t!(args, ARG_PEER_ID, String).ok();

        let config_path =
            value_t!(args, ARG_CONFIG_PATH, String).expect("Path to config file must be specified");
        info!("Loading node config from: {}", &config_path);
        node_config = NodeConfig::load_config(peer_id, &config_path).expect("NodeConfig");

        info!("Starting Full {}", node_config.base.peer_id);
    } else {
        // Note we will silently ignore --peer_id arg here
        // 注意，我们将在此处默默忽略--peer_id arg
        info!("Loading test configs");
        node_config = NodeConfigHelpers::get_single_node_test_config(false /* random ports */);

        info!("Starting Single-Mode {}", node_config.base.peer_id);
    }

    // Node configuration contains important ephemeral port information and should
    // not be subject to being disabled as with other logs
    // 节点配置包含重要的临时端口信息，不应像其他日志一样被禁用
    println!("Using node config {:?}", &node_config);

    node_config
}

// 设置指标
pub fn setup_metrics(peer_id: &str, node_config: &NodeConfig) {
    if !node_config.metrics.dir.as_os_str().is_empty() {
        metrics::dump_all_metrics_to_file_periodically(
            &node_config.metrics.dir,
            &format!("{}.metrics", peer_id),
            node_config.metrics.collection_interval_ms,
        );
    }

    // TODO: should we do this differently for different binaries?
    if !node_config.metrics.push_server_addr.is_empty() {
        metrics::push_all_metrics_to_pushgateway_periodically(
            "libra_node",
            &node_config.metrics.push_server_addr,
            peer_id,
            node_config.metrics.collection_interval_ms,
        );
    }
}

/// Performs common setup for the executable.  Takes in args that
/// you wish to use for this executable
/// 执行可执行文件的常见设置。 接受您希望用于此可执行文件的参数
pub fn setup_executable(
    app_name: String,
    arg_names: Vec<&str>,
) -> (NodeConfig, Option<GlobalLoggerGuard>, ArgMatches<'_>) {
    crash_handler::setup_panic_handler();

    let args = get_arg_matches(app_name, arg_names);
    let is_logging_disabled = args.is_present(ARG_DISABLE_LOGGING);
    let mut _logger = set_default_global_logger(is_logging_disabled, None);

    let config = load_configs_from_args(&args);

    // Reset the global logger using config (for chan_size currently).
    // We need to drop the global logger guard first before resetting it.
    // 使用config重置全局记录器（当前为chan_size）。我们需要在重置之前先删除全局记录器保护。
    _logger = None;
    let logger = set_default_global_logger(
        is_logging_disabled,
        Some(config.base.node_async_log_chan_size),
    );

    setup_metrics(&config.base.peer_id, &config);

    (config, logger, args)
}

fn set_default_global_logger(
    is_logging_disabled: bool,
    chan_size: Option<usize>,
) -> Option<GlobalLoggerGuard> {
    if is_logging_disabled {
        return None;
    }

    Some(logger::set_default_global_logger(
        true,      /* async */
        chan_size, /* chan_size */
    ))
}

fn get_arg_matches(app_name: String, arg_names: Vec<&str>) -> ArgMatches<'_> {
    let mut service_name = app_name.clone();
    service_name.push_str(" Service");

    let mut app = App::new(app_name)
        .version("0.1.0")
        .author("Libra Association <opensource@libra.org>")
        .about(service_name.as_str());

    for arg in arg_names {
        let short;
        let takes_value;
        let help;
        match arg {
            ARG_PEER_ID => {
                short = "-p";
                takes_value = true;
                help = "Specify peer id for this node";
            }
            ARG_CONFIG_PATH => {
                short = "-f";
                takes_value = true;
                help = "Specify the path to the config file";
            }
            ARG_DISABLE_LOGGING => {
                short = "-d";
                takes_value = false;
                help = "Controls logging";
            }
            ARG_NUM_PAYLOAD => {
                short = "-n";
                takes_value = true;
                help = "Specify the number of payload each node send";
            }
            ARG_PAYLOAD_SIZE => {
                short = "-s";
                takes_value = true;
                help = "Specify the byte size of each payload";
            }
            x => panic!("Invalid argument: {}", x),
        }
        app = app.arg(
            Arg::with_name(arg)
                .short(short)
                .long(arg)
                .takes_value(takes_value)
                .help(help),
        );
    }

    app.get_matches()
}
