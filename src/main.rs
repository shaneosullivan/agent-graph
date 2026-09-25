use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    // `emit` runs inside provider hooks, so it skips clap: a usage error there
    // would exit with code 2, which Claude Code treats as "block this action".
    if args.get(1).is_some_and(|a| a == "emit") {
        agent_graph::emit::run_and_exit(&args[2..]);
    }
    agent_graph::cli::run()
}
