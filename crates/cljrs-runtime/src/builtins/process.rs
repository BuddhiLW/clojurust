//! Process control and standard input: `System/exit` and `read-line`.
//!
//! `System/getenv` is the third of this family and lives in `builtins.rs`; the
//! `System/…` naming convention is documented beside it in the crate README.
use cljrs_value::{Value, ValueError, ValueResult};
use std::io::{BufRead, Write};

/// `(System/exit status)` terminates the process with `status`.
///
/// Never returns, so it has no place in a value position, and nothing after it
/// in the program runs.
pub(crate) fn builtin_exit(args: &[Value]) -> ValueResult<Value> {
    let status = match &args[0] {
        Value::Long(n) => i32::try_from(*n).map_err(|_| ValueError::OutOfRange)?,
        value => {
            return Err(ValueError::WrongType {
                expected: "integer",
                got: value.type_name().to_string(),
            });
        }
    };
    // process::exit skips destructors, including buffered output cleanup: what
    // a program printed before exiting must still reach the pipe.
    std::io::stdout()
        .flush()
        .map_err(|e| ValueError::Other(e.to_string()))?;
    std::io::stderr()
        .flush()
        .map_err(|e| ValueError::Other(e.to_string()))?;
    // On Linux, exit runs TLS destructors on its calling thread. The eval
    // thread may be inside Tokio's scheduler, whose TLS cannot be destroyed
    // while its core is checked out ("Oh no! We never placed the Core back"),
    // and the abort that follows loses the status. Exit from a fresh thread
    // with no runtime TLS instead; process::exit still terminates every thread.
    std::thread::Builder::new()
        .name("cljrs-exit".into())
        .spawn(move || std::process::exit(status))
        .map_err(|e| ValueError::Other(e.to_string()))?
        .join()
        .map_err(|_| ValueError::Other("exit thread panicked".into()))?;
    unreachable!("process::exit never returns")
}

/// `(read-line)` reads one line from stdin, without its terminator, or `nil` at
/// end of input. `\r\n` and `\n` both terminate a line; an unterminated final
/// line is still a line.
pub(crate) fn builtin_read_line(_args: &[Value]) -> ValueResult<Value> {
    let mut line = String::new();
    let bytes = std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| ValueError::Other(e.to_string()))?;
    if bytes == 0 {
        return Ok(Value::Nil);
    }
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    Ok(Value::string(line))
}
