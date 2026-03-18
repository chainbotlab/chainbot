//! [INPUT]
//! Process startup state and command-line arguments.
//!
//! [OUTPUT]
//! Invokes the CLI runtime and terminates the process with stable user-facing exit codes.
//!
//! [ROLE]
//! Serves as the binary entrypoint for the `chainbot` executable.

fn main() -> std::process::ExitCode {
    match chainbot::cli::run_from_env() {
        Ok(output) => {
            if !output.stdout().is_empty() {
                print!("{}", output.stdout());
            }
            std::process::ExitCode::from(chainbot::errors::CliExitCode::Success.as_u8())
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::from(error.exit_code().as_u8())
        }
    }
}
