// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use client::{client_proxy::ClientProxy, commands::*};
use logger::set_default_global_logger;
use rustyline::{config::CompletionType, error::ReadlineError, Config, Editor};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(
    name = "Libra Client",
    author = "The Libra Association",
    about = "Libra client to connect to a specific validator"
)]
struct Args {
    /// Admission Control port to connect to. 连接AC的端口
    #[structopt(short = "p", long = "port", default_value = "30307")]
    pub port: String,
    /// Host address/name to connect to. 连接的主机地址/名字
    #[structopt(short = "a", long = "host")]
    pub host: String,
    /// Path to the generated keypair for the faucet account. The faucet account can be used to
    /// mint coins. If not passed, a new keypair will be generated for
    /// you and placed in a temporary directory.
    /// To manually generate a keypair, use generate_keypair:
    /// `cargo run -p generate_keypair -- -o <output_file_path>`
    /// 用于为 faucet账户生成 keypair的路径。faucet 账户可以铸币。如果不通过，一个新的keypair可以
    /// 被生成放到临时目录。人工生成keypair 使用 generate_keypair程序
    #[structopt(short = "m", long = "faucet_key_file_path")]
    pub faucet_account_file: Option<String>,
    /// Host that operates a faucet service
    /// If not passed, will be derived from host parameter
    /// Host 操作一个faucet服务
    /// 如果没有，从host parameter继承
    #[structopt(short = "f", long = "faucet_server")]
    pub faucet_server: Option<String>,
    /// File location from which to load mnemonic word for user account address/key generation.
    /// If not passed, a new mnemonic file will be generated by libra_wallet in the current
    /// directory.
    /// 用于加载用户帐户地址/密钥生成的助记词的文件位置。如果没有，当前目录中的libra_wallet
    /// 将生成一个新的助记符文件
    #[structopt(short = "n", long = "mnemonic_file")]
    pub mnemonic_file: Option<String>,
    /// File location from which to load config of trusted validators. It is used to verify
    /// validator signatures in validator query response. The file should at least include public
    /// key of all validators trusted by the client - which should typically be all validators on
    /// the network. To connect to testnet, use 'libra/scripts/cli/trusted_peers.config.toml'.
    /// Can be generated by libra-config for local testing:
    /// `cargo run --bin libra-config`
    /// But the preferred method is to simply use libra-swarm to run local networks
    /// 文件位于加载信任验证者配置的地方。在验证者查询回应中被用来验证验证者签名。这个文件至少包含所有
    /// 被客户端信任的验证者的共钥-通常应该是网络上的所有验证器。连接到 testnet，使用'libra/scripts/cli/trusted_peers.config.toml'.
    /// 可以使用libra-config为本地测试生成：
    ///`cargo run --bin libra-config`
    ///但首选的方法是简单地使用libra-swarm来运行本地网络
    #[structopt(short = "s", long = "validator_set_file")]
    pub validator_set_file: String,
    /// If set, client will sync with validator during wallet recovery.
    /// 如果设置，客户端将在钱包恢复期间与验证器同步。
    #[structopt(short = "r", long = "sync")]
    pub sync: bool,
}

/// 测试 --host ac.testnet.libra.org --port 8000 -s $SCRIPT_PATH/trusted_peers.config.toml
fn main() -> std::io::Result<()> {
    let _logger = set_default_global_logger(false /* async */, None);
    crash_handler::setup_panic_handler();

    let (commands, alias_to_cmd) = get_commands();

    let args = Args::from_args();
    let faucet_account_file = args.faucet_account_file.unwrap_or_else(|| "".to_string());

    let mut client_proxy = ClientProxy::new(
        &args.host,
        &args.port,
        &args.validator_set_file,
        &faucet_account_file,
        args.sync,
        args.faucet_server,
        args.mnemonic_file,
    )
    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, &format!("{}", e)[..]))?;

    // Test connection to validator
    let test_ret = client_proxy.test_validator_connection();

    if let Err(e) = test_ret {
        println!(
            "Not able to connect to validator at {}:{}, error {:?}",
            args.host, args.port, e
        );
        return Ok(());
    }
    let cli_info = format!("Connected to validator at: {}:{}", args.host, args.port);
    print_help(&cli_info, &commands);
    println!("Please, input commands: \n");

    let config = Config::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .auto_add_history(true)
        .build();
    let mut rl = Editor::<()>::with_config(config);
    loop {
        let readline = rl.readline("libra% ");
        match readline {
            Ok(line) => {
                let params = parse_cmd(&line);
                match alias_to_cmd.get(params[0]) {
                    Some(cmd) => cmd.execute(&mut client_proxy, &params),
                    None => match params[0] {
                        "quit" | "q!" => break,
                        "help" | "h" => print_help(&cli_info, &commands),
                        "" => continue,
                        x => println!("Unknown command: {:?}", x),
                    },
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }

    Ok(())
}

/// Print the help message for the client and underlying command.
fn print_help(client_info: &str, commands: &[std::sync::Arc<dyn Command>]) {
    println!("{}", client_info);
    println!("usage: <command> <args>\n\nUse the following commands:\n");
    for cmd in commands {
        println!(
            "{} {}\n\t{}",
            cmd.get_aliases().join(" | "),
            cmd.get_params_help(),
            cmd.get_description()
        );
    }

    println!("help | h \n\tPrints this help");
    println!("quit | q! \n\tExit this client");
    println!("\n");
}
