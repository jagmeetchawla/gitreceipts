//! Paging the console report through `$PAGER`, git-style: on a terminal
//! the report flows through a pager with colors intact; a pipe or redirect
//! is never paged.

/// Start `$PAGER` when writing to a terminal and redirect our stdout to it,
/// so every `println!` flows to the pager without threading a writer
/// through the report code. Returns the child (kept alive until
/// [`finish_pager`]); `None` means output goes straight to stdout.
#[cfg(unix)]
pub fn start_pager(no_pager: bool) -> Option<std::process::Child> {
    use std::io::IsTerminal;
    use std::os::unix::io::AsRawFd;
    use std::process::{Command, Stdio};

    if no_pager || !std::io::stdout().is_terminal() {
        return None;
    }
    let pager = std::env::var("GIT_PAGER")
        .or_else(|_| std::env::var("PAGER"))
        .unwrap_or_else(|_| "less".to_string());
    if pager.trim().is_empty() || pager.trim() == "cat" {
        return None;
    }
    // git's defaults: R = render ANSI color, F = quit if one screen,
    // X = don't clear the screen on exit.
    if std::env::var_os("LESS").is_none() {
        unsafe { std::env::set_var("LESS", "FRX") };
    }
    let child = Command::new("sh")
        .arg("-c")
        .arg(&pager)
        .stdin(Stdio::piped())
        .spawn()
        .ok()?;
    let fd = child.stdin.as_ref()?.as_raw_fd();
    unsafe { libc::dup2(fd, libc::STDOUT_FILENO) };
    Some(child)
}

/// Close the pipe to the pager (both fd copies) so it sees EOF, then wait
/// for the user to quit it.
#[cfg(unix)]
pub fn finish_pager(child: Option<std::process::Child>) {
    use std::io::Write;
    if let Some(mut child) = child {
        let _ = std::io::stdout().flush();
        drop(child.stdin.take());
        unsafe { libc::close(libc::STDOUT_FILENO) };
        let _ = child.wait();
    }
}

#[cfg(not(unix))]
pub fn start_pager(_no_pager: bool) -> Option<std::process::Child> {
    None
}

#[cfg(not(unix))]
pub fn finish_pager(_child: Option<std::process::Child>) {}
