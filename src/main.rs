use otterlang::cli;

fn main() -> anyhow::Result<()> {
    if let Err(e) = cli::run() {
        let msg = e.to_string();
        // If the error is a known compilation failure that has already emitted diagnostics,
        // just exit with error code 1 without printing the error object (which creates noise).
        if msg.contains("lexing failed")
            || msg.contains("parsing failed")
            || msg.contains("type checking failed")
        {
            std::process::exit(1);
        }
        // For other unexpected errors, print them.
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
    fn build_command_honors_output_flag() {
        let cli = OtterCli::parse_from([
            "otter",
            "build",
            "examples/app.ot",
            "--output",
            "target/app",
        ]);
        match cli.command() {
            Command::Build { path, output } => {
                assert_eq!(path.to_string_lossy(), "examples/app.ot");
                assert_eq!(
                    output.as_ref().map(|p| p.to_string_lossy().into_owned()),
                    Some("target/app".into())
                );
            }
            other => panic!("expected build command, got {other:?}"),
        }
    }
}
