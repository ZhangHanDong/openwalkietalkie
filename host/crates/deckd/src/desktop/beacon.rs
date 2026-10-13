//! Refresh the terminal title through the hook's recorded tty output.
//! No text is fed into the CLI: the only bytes written are an OSC 2 title
//! with a fresh random nonce. The caller checks the CLI's foreground/tty
//! identity first, then matches that nonce in Ghostty's surface list.
use super::{ghostty, DesktopError, Target};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};

pub(super) fn refresh(target: &Target) -> Result<String, DesktopError> {
    let nonce = format!(
        "{:016x}",
        getrandom::u64().map_err(|_| DesktopError::NoTerminal)?
    );
    write_title(&target.tty, &nonce)?;
    Ok(nonce)
}

fn write_title(tty: &str, nonce: &str) -> Result<(), DesktopError> {
    let mut file = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(tty)
        .map_err(|_| DesktopError::NoTerminal)?;
    if !file.metadata().is_ok_and(|m| m.file_type().is_char_device())
        // SAFETY: `file` owns this live descriptor throughout the call.
        || unsafe { libc::isatty(file.as_raw_fd()) } != 1
    {
        return Err(DesktopError::NoTerminal);
    }
    file.write_all(ghostty::beacon(nonce).as_bytes())
        .map_err(|_| DesktopError::NoTerminal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::fd::FromRawFd;

    #[test]
    fn a_title_is_terminal_output_and_never_cli_input() {
        let (mut master, mut slave) = (-1, -1);
        let mut name = [0; libc::PATH_MAX as usize];
        // SAFETY: all output buffers are live, and null requests default
        // terminal settings. This allocates a private pair for this test.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    name.as_mut_ptr(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            },
            0
        );
        // SAFETY: openpty transferred ownership of these two descriptors.
        let (mut master, mut slave) = unsafe {
            (
                std::fs::File::from_raw_fd(master),
                std::fs::File::from_raw_fd(slave),
            )
        };
        // SAFETY: openpty filled the path buffer with a NUL-terminated name.
        let tty = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }
            .to_str()
            .unwrap();
        write_title(tty, "test-nonce").unwrap();
        let expected = ghostty::beacon("test-nonce");
        let mut output = vec![0; expected.len()];
        master.read_exact(&mut output).unwrap();
        assert_eq!(output, expected.as_bytes());
        // SAFETY: the slave descriptor remains owned and open here.
        assert_eq!(
            unsafe { libc::fcntl(slave.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) },
            0
        );
        assert_eq!(
            slave.read(&mut [0; 1]).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn a_regular_file_or_symlink_cannot_receive_a_title() {
        let dir = crate::core::testkit::temp("beacon-non-tty");
        let path = dir.join("not-a-terminal");
        std::fs::write(&path, "keep this file").unwrap();
        assert_eq!(
            write_title(path.to_str().unwrap(), "abcd"),
            Err(DesktopError::NoTerminal)
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep this file");
        let link = dir.join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(
            write_title(link.to_str().unwrap(), "abcd"),
            Err(DesktopError::NoTerminal)
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
