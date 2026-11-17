//! Print the user guide embedded at build time.
//!
//! The guide is the same `docs/USER-GUIDE.md` that is distributed with the project, compiled
//! into the binary so an appliance without the repository can read it. The output is raw
//! Markdown; a pager or renderer is the operator's choice and is never spawned here.

use std::io::{self, ErrorKind, IsTerminal, Write};

use thiserror::Error;

/// The user guide of this build.
pub const USER_GUIDE: &str = include_str!("../docs/USER-GUIDE.md");

/// Why the manual could not be written.
#[derive(Debug, Error)]
pub enum ManualError {
    #[error("cannot write the manual to standard output: {source}")]
    Write { source: io::Error },
}

/// Print the guide to standard output.
pub fn run() -> Result<(), ManualError> {
    let stdout = io::stdout();
    if stdout.is_terminal() {
        let _ = writeln!(
            io::stderr(),
            "hint: this is Markdown; pipe it to a pager (less, more) or mdcat --ansi"
        );
    }
    let mut out = stdout.lock();
    write_guide(&mut out)
}

/// Write the guide to `out`.
///
/// A consumer that closes the pipe early (`manual | head`) is a success, not an error; any
/// other write failure is reported.
pub fn write_guide<W: Write>(out: &mut W) -> Result<(), ManualError> {
    match out
        .write_all(USER_GUIDE.as_bytes())
        .and_then(|()| out.flush())
    {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == ErrorKind::BrokenPipe => Ok(()),
        Err(source) => Err(ManualError::Write { source }),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use super::*;

    struct BrokenPipe;

    impl Write for BrokenPipe {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(ErrorKind::BrokenPipe, "closed"))
        }
    }

    struct OtherError;

    impl Write for OtherError {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("boom"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_embedded_guide_is_the_user_guide() {
        assert!(USER_GUIDE.starts_with("# User Guide"));
        for heading in [
            "## Install",
            "## Configuration",
            "## Command line",
            "## Limits and deliberate choices",
        ] {
            assert!(USER_GUIDE.contains(heading), "missing {heading}");
        }
    }

    #[test]
    fn the_guide_is_written_whole() {
        let mut out = Vec::new();

        write_guide(&mut out).expect("the guide is written");

        assert_eq!(out, USER_GUIDE.as_bytes());
    }

    #[test]
    fn a_broken_pipe_is_success() {
        assert!(write_guide(&mut BrokenPipe).is_ok());
    }

    #[test]
    fn another_write_error_is_reported() {
        assert!(matches!(
            write_guide(&mut OtherError),
            Err(ManualError::Write { .. })
        ));
    }
}
