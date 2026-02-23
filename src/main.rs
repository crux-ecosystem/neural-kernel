//! Neural Kernel — CLI entry point
//!
//! Commands:
//!   nk start              Start the kernel
//!   nk agent spawn        Spawn an agent
//!   nk agent list         List running agents
//!   nk agent kill <id>    Kill an agent
//!   nk fs mount           Mount VectorFS
//!   nk status             Kernel status
//!   nk bench              Run benchmarks

use std::process::ExitCode;
use tracing_subscriber::{fmt, EnvFilter};

fn main() -> ExitCode {
    // Initialize tracing
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(true)
        .with_thread_ids(true)
        .init();

    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        return ExitCode::SUCCESS;
    }

    match args[1].as_str() {
        "start" => cmd_start(&args[2..]),
        "status" => cmd_status(),
        "agent" if args.len() > 2 => match args[2].as_str() {
            "spawn" => cmd_agent_spawn(&args[3..]),
            "list" => cmd_agent_list(),
            "kill" if args.len() > 3 => cmd_agent_kill(&args[3]),
            _ => {
                eprintln!("Unknown agent command: {}", args[2]);
                ExitCode::FAILURE
            }
        },
        "fs" if args.len() > 2 => match args[2].as_str() {
            "mount" => cmd_fs_mount(&args[3..]),
            "unmount" => cmd_fs_unmount(),
            _ => {
                eprintln!("Unknown fs command: {}", args[2]);
                ExitCode::FAILURE
            }
        },
        "version" | "--version" | "-V" => {
            println!(
                "Neural Kernel v{} ({})",
                env!("CARGO_PKG_VERSION"),
                env!("CARGO_PKG_REPOSITORY")
            );
            ExitCode::SUCCESS
        }
        "help" | "--help" | "-h" => {
            print_usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("Unknown command: {other}");
            print_usage();
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    println!(
        r#"
Neural Kernel — AI Microkernel for the Sovereign Stack

USAGE:
    nk <COMMAND> [OPTIONS]

COMMANDS:
    start               Start the kernel runtime
    status              Show kernel status
    agent spawn <wasm>  Spawn a new agent
    agent list          List running agents
    agent kill <id>     Terminate an agent
    fs mount <path>     Mount VectorFS at path
    fs unmount          Unmount VectorFS
    version             Show version
    help                Show this help

OPTIONS:
    --config <file>     Config file path (default: ~/.nk/config.toml)
    --log-level <lvl>   Log level: trace, debug, info, warn, error

Part of the Crux Sovereign AI Stack
https://github.com/crux-ecosystem
"#
    );
}

// ─── Command Handlers ──────────────────────────────────────

fn cmd_start(args: &[String]) -> ExitCode {
    tracing::info!("Starting Neural Kernel...");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("nk-worker")
        .build()
        .expect("Failed to build Tokio runtime");

    rt.block_on(async {
        let config = neural_kernel::SchedulerConfig::default();
        let scheduler = neural_kernel::Scheduler::new(config);

        tracing::info!(
            max_agents = scheduler.config.max_agents,
            "Kernel initialized"
        );

        // Keep running until signal
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for ctrl+c");
        tracing::info!("Shutting down...");
    });

    ExitCode::SUCCESS
}

fn cmd_status() -> ExitCode {
    println!("Neural Kernel Status:");
    println!("  Version:  {}", env!("CARGO_PKG_VERSION"));
    println!("  Status:   Idle (no active kernel)");
    println!("  Agents:   0");
    println!("  VectorFS: Not mounted");
    ExitCode::SUCCESS
}

fn cmd_agent_spawn(args: &[String]) -> ExitCode {
    if args.is_empty() {
        eprintln!("Usage: nk agent spawn <binary|wasm>");
        return ExitCode::FAILURE;
    }
    println!("Spawning agent from: {}", args[0]);
    // TODO: Implement actual agent spawning
    ExitCode::SUCCESS
}

fn cmd_agent_list() -> ExitCode {
    println!("No running agents");
    ExitCode::SUCCESS
}

fn cmd_agent_kill(id: &str) -> ExitCode {
    println!("Killing agent: {id}");
    // TODO: Implement
    ExitCode::SUCCESS
}

fn cmd_fs_mount(args: &[String]) -> ExitCode {
    let mount_point = args.first().map(|s| s.as_str()).unwrap_or("/tmp/vectorfs");
    println!("Mounting VectorFS at: {mount_point}");
    // TODO: Implement FUSE mount
    ExitCode::SUCCESS
}

fn cmd_fs_unmount() -> ExitCode {
    println!("Unmounting VectorFS");
    ExitCode::SUCCESS
}
