fn main() -> std::process::ExitCode {
    match code_index::cli::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            code_index::log::error(&err.to_string());
            std::process::ExitCode::FAILURE
        }
    }
}
