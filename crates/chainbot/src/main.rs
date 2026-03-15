/*
[INPUT]:  Process startup context and command line arguments.
[OUTPUT]: Executes the CLI command surface and returns explicit success or failure exit codes.
[POS]:    Binary entrypoint for chainbot CLI execution.
[UPDATE]: 2026-03-16 - Wire validate command execution and error exit path.
[UPDATE]: 2026-03-16 - Route help/stdout output and user-facing CLI exit codes.
*/
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
