use otterlang::cli;

fn main() -> anyhow::Result<()> {
    if let Err(e) = cli::run() {
        let msg = e.to_string();
        if msg.contains("lexing failed")
            || msg.contains("parsing failed")
            || msg.contains("type checking failed")
        {
            std::process::exit(1);
        }
        #[expect(clippy::print_stderr, reason = "TODO: Use robust logging")]
        {
            eprintln!("Error: {}", msg);
        }
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic, reason = "Panicking on test failures is acceptable")]

    use clap::Parser;
    use otterlang::cli::{Command, OtterCli};

    #[test]
    fn run_command_parses_path_argument() {
        let cli = OtterCli::parse_from(["otter", "run", "tests/demo.ot"]); // no filesystem access
        match cli.command() {
            Command::Run { path } => assert_eq!(path.to_string_lossy(), "tests/demo.ot"),
            other => panic!("expected run command, got {other:?}"),
        }
    }
}
