//! Subprocess Management
//!
//! This module provides subprocess spawning and management for Blood programs.
//! It's essential for the self-hosted compiler to invoke linkers and assemblers.
//!
//! # Features
//!
//! - **Process Spawning**: Create child processes with arguments
//! - **I/O Redirection**: Capture or pipe stdin/stdout/stderr
//! - **Environment**: Set environment variables for child processes
//! - **Working Directory**: Set the working directory for child processes
//! - **Exit Status**: Wait for and retrieve exit codes
//!
//! # Example
//!
//! ```rust,ignore
//! use blood_runtime::process::{Command, Stdio};
//!
//! // Simple command execution
//! let output = Command::new("echo")
//!     .arg("Hello, World!")
//!     .output()?;
//!
//! assert!(output.status.success());
//! println!("{}", String::from_utf8_lossy(&output.stdout));
//!
//! // Complex pipeline
//! let mut child = Command::new("gcc")
//!     .args(&["-c", "foo.c", "-o", "foo.o"])
//!     .current_dir("/tmp/build")
//!     .env("CC", "gcc")
//!     .stdin(Stdio::Null)
//!     .stdout(Stdio::Piped)
//!     .stderr(Stdio::Piped)
//!     .spawn()?;
//!
//! let status = child.wait()?;
//! ```

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{self, ExitStatus, Stdio as StdStdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Get the configured compute timeout from the runtime configuration.
///
/// Returns the default (30 seconds) if no runtime config is available.
pub fn configured_compute_timeout() -> Duration {
    crate::runtime_config()
        .map(|c| c.timeout.compute_timeout)
        .unwrap_or_else(|| Duration::from_secs(30))
}

/// Counter for generating unique process IDs.
static PROCESS_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// A process builder, providing fine-grained control over how a new process
/// should be spawned.
#[derive(Debug, Clone)]
pub struct Command {
    /// Program to execute.
    program: OsString,
    /// Arguments to pass to the program.
    args: Vec<OsString>,
    /// Environment variables (None means inherit).
    env: Option<HashMap<OsString, OsString>>,
    /// Environment variables to add/override.
    env_additions: HashMap<OsString, OsString>,
    /// Environment variables to remove.
    env_removals: Vec<OsString>,
    /// Working directory.
    current_dir: Option<PathBuf>,
    /// Stdin configuration.
    stdin: Option<Stdio>,
    /// Stdout configuration.
    stdout: Option<Stdio>,
    /// Stderr configuration.
    stderr: Option<Stdio>,
}

impl Command {
    /// Create a new command builder for the given program.
    pub fn new<S: AsRef<OsStr>>(program: S) -> Self {
        Self {
            program: program.as_ref().to_os_string(),
            args: Vec::new(),
            env: None,
            env_additions: HashMap::new(),
            env_removals: Vec::new(),
            current_dir: None,
            stdin: None,
            stdout: None,
            stderr: None,
        }
    }

    /// Add an argument to the command.
    pub fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    /// Add multiple arguments to the command.
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for arg in args {
            self.args.push(arg.as_ref().to_os_string());
        }
        self
    }

    /// Set an environment variable for the child process.
    pub fn env<K, V>(&mut self, key: K, val: V) -> &mut Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.env_additions
            .insert(key.as_ref().to_os_string(), val.as_ref().to_os_string());
        self
    }

    /// Set multiple environment variables.
    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        for (key, val) in vars {
            self.env_additions
                .insert(key.as_ref().to_os_string(), val.as_ref().to_os_string());
        }
        self
    }

    /// Remove an environment variable from the child process.
    pub fn env_remove<K: AsRef<OsStr>>(&mut self, key: K) -> &mut Self {
        self.env_removals.push(key.as_ref().to_os_string());
        self
    }

    /// Clear all environment variables (don't inherit from parent).
    pub fn env_clear(&mut self) -> &mut Self {
        self.env = Some(HashMap::new());
        self
    }

    /// Set the working directory for the child process.
    pub fn current_dir<P: AsRef<Path>>(&mut self, dir: P) -> &mut Self {
        self.current_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Set the stdin configuration.
    pub fn stdin<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.stdin = Some(cfg.into());
        self
    }

    /// Set the stdout configuration.
    pub fn stdout<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.stdout = Some(cfg.into());
        self
    }

    /// Set the stderr configuration.
    pub fn stderr<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.stderr = Some(cfg.into());
        self
    }

    /// Get the program name.
    pub fn get_program(&self) -> &OsStr {
        &self.program
    }

    /// Get the arguments.
    pub fn get_args(&self) -> impl Iterator<Item = &OsStr> {
        self.args.iter().map(|s| s.as_os_str())
    }

    /// Get the current directory setting.
    pub fn get_current_dir(&self) -> Option<&Path> {
        self.current_dir.as_deref()
    }

    /// Build a std::process::Command from this builder.
    fn to_std_command(&self) -> process::Command {
        let mut cmd = process::Command::new(&self.program);

        cmd.args(&self.args);

        // Handle environment
        if let Some(ref env) = self.env {
            cmd.env_clear();
            for (k, v) in env {
                cmd.env(k, v);
            }
        }
        for (k, v) in &self.env_additions {
            cmd.env(k, v);
        }
        for k in &self.env_removals {
            cmd.env_remove(k);
        }

        if let Some(ref dir) = self.current_dir {
            cmd.current_dir(dir);
        }

        if let Some(ref stdin) = self.stdin {
            cmd.stdin(stdin.to_std());
        }
        if let Some(ref stdout) = self.stdout {
            cmd.stdout(stdout.to_std());
        }
        if let Some(ref stderr) = self.stderr {
            cmd.stderr(stderr.to_std());
        }

        cmd
    }

    /// Spawn the command, returning a handle to the child process.
    pub fn spawn(&mut self) -> io::Result<Child> {
        let child = self.to_std_command().spawn()?;
        let id = PROCESS_ID_COUNTER.fetch_add(1, Ordering::SeqCst);

        Ok(Child {
            id,
            inner: child,
            program: self.program.clone(),
        })
    }

    /// Execute the command and wait for it to finish.
    ///
    /// Returns the exit status.
    pub fn status(&mut self) -> io::Result<ExitStatus> {
        self.to_std_command().status()
    }

    /// Execute the command and collect its output.
    ///
    /// Captures stdout and stderr.
    pub fn output(&mut self) -> io::Result<Output> {
        let output = self.to_std_command().output()?;
        Ok(Output {
            status: ProcessStatus::from_exit_status(output.status),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// Describes what to do with a standard I/O stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stdio {
    /// Inherit from the parent process.
    Inherit,
    /// Redirect to /dev/null (or NUL on Windows).
    Null,
    /// Create a pipe for reading/writing.
    Piped,
}

impl Stdio {
    fn to_std(&self) -> StdStdio {
        match self {
            Stdio::Inherit => StdStdio::inherit(),
            Stdio::Null => StdStdio::null(),
            Stdio::Piped => StdStdio::piped(),
        }
    }
}

impl Default for Stdio {
    fn default() -> Self {
        Stdio::Inherit
    }
}

/// A handle to a running child process.
#[derive(Debug)]
pub struct Child {
    /// Unique ID for this child process.
    id: u64,
    /// The underlying process.
    inner: process::Child,
    /// Program name (for error messages).
    #[allow(dead_code)]
    program: OsString,
}

impl Child {
    /// Get the unique ID for this child process.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Get the OS-assigned process ID.
    pub fn pid(&self) -> u32 {
        self.inner.id()
    }

    /// Take the child's stdin handle.
    pub fn stdin(&mut self) -> Option<ChildStdin> {
        self.inner.stdin.take().map(|s| ChildStdin { inner: s })
    }

    /// Take the child's stdout handle.
    pub fn stdout(&mut self) -> Option<ChildStdout> {
        self.inner.stdout.take().map(|s| ChildStdout { inner: s })
    }

    /// Take the child's stderr handle.
    pub fn stderr(&mut self) -> Option<ChildStderr> {
        self.inner.stderr.take().map(|s| ChildStderr { inner: s })
    }

    /// Wait for the child to exit.
    pub fn wait(&mut self) -> io::Result<ProcessStatus> {
        let status = self.inner.wait()?;
        Ok(ProcessStatus::from_exit_status(status))
    }

    /// Check if the child has exited without blocking.
    pub fn try_wait(&mut self) -> io::Result<Option<ProcessStatus>> {
        match self.inner.try_wait()? {
            Some(status) => Ok(Some(ProcessStatus::from_exit_status(status))),
            None => Ok(None),
        }
    }

    /// Kill the child process.
    pub fn kill(&mut self) -> io::Result<()> {
        self.inner.kill()
    }

    /// Wait for the child and collect its output.
    pub fn wait_with_output(self) -> io::Result<Output> {
        let output = self.inner.wait_with_output()?;
        Ok(Output {
            status: ProcessStatus::from_exit_status(output.status),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    /// Wait for the child to exit with a timeout.
    ///
    /// Returns `Ok(None)` if the timeout elapsed before the child exited.
    /// Returns `Ok(Some(status))` if the child exited within the timeout.
    ///
    /// Note: If the timeout elapses, the child process is still running.
    /// Call `kill()` if you want to terminate it.
    pub fn wait_timeout(&mut self, timeout: Duration) -> io::Result<Option<ProcessStatus>> {
        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(10);

        loop {
            // Check if child has exited
            if let Some(status) = self.try_wait()? {
                return Ok(Some(status));
            }

            // Check if timeout has elapsed
            if start.elapsed() >= timeout {
                return Ok(None);
            }

            // Sleep for a short interval before checking again
            std::thread::sleep(poll_interval);
        }
    }

    /// Wait for the child to exit using the configured compute timeout.
    ///
    /// Uses `config.timeout.compute_timeout` from the runtime configuration,
    /// falling back to 30 seconds if no configuration is available.
    ///
    /// Returns `Ok(None)` if the timeout elapsed before the child exited.
    /// Returns `Ok(Some(status))` if the child exited within the timeout.
    pub fn wait_with_configured_timeout(&mut self) -> io::Result<Option<ProcessStatus>> {
        self.wait_timeout(configured_compute_timeout())
    }

    /// Wait for the child to exit with timeout, killing it if timeout elapses.
    ///
    /// Unlike `wait_timeout`, this method will kill the child process if it
    /// doesn't exit within the timeout, then wait for it to actually terminate.
    pub fn wait_timeout_or_kill(&mut self, timeout: Duration) -> io::Result<ProcessStatus> {
        match self.wait_timeout(timeout)? {
            Some(status) => Ok(status),
            None => {
                // Kill the child and wait for it to terminate
                self.kill()?;
                self.wait()
            }
        }
    }

    /// Wait with configured timeout, killing the child if timeout elapses.
    ///
    /// Uses `config.timeout.compute_timeout` from the runtime configuration.
    pub fn wait_with_configured_timeout_or_kill(&mut self) -> io::Result<ProcessStatus> {
        self.wait_timeout_or_kill(configured_compute_timeout())
    }
}

/// Handle to a child process's stdin.
#[derive(Debug)]
pub struct ChildStdin {
    inner: process::ChildStdin,
}

impl Write for ChildStdin {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Handle to a child process's stdout.
#[derive(Debug)]
pub struct ChildStdout {
    inner: process::ChildStdout,
}

impl Read for ChildStdout {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

/// Handle to a child process's stderr.
#[derive(Debug)]
pub struct ChildStderr {
    inner: process::ChildStderr,
}

impl Read for ChildStderr {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

/// The status of a completed process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessStatus {
    /// Exit code (None if terminated by signal).
    code: Option<i32>,
    /// Signal that terminated the process (Unix only).
    #[cfg(unix)]
    signal: Option<i32>,
}

impl ProcessStatus {
    /// Create a status from an exit code.
    pub fn from_code(code: i32) -> Self {
        Self {
            code: Some(code),
            #[cfg(unix)]
            signal: None,
        }
    }

    /// Create from std::process::ExitStatus.
    fn from_exit_status(status: ExitStatus) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            Self {
                code: status.code(),
                signal: status.signal(),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                code: status.code(),
            }
        }
    }

    /// Check if the process exited successfully (exit code 0).
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// Get the exit code, if the process exited normally.
    pub fn code(&self) -> Option<i32> {
        self.code
    }

    /// Get the signal that terminated the process (Unix only).
    #[cfg(unix)]
    pub fn signal(&self) -> Option<i32> {
        self.signal
    }
}

impl std::fmt::Display for ProcessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.code {
            Some(code) => write!(f, "exit code: {}", code),
            None => {
                #[cfg(unix)]
                if let Some(sig) = self.signal {
                    return write!(f, "signal: {}", sig);
                }
                write!(f, "unknown")
            }
        }
    }
}

/// The output of a finished process.
#[derive(Debug, Clone)]
pub struct Output {
    /// The exit status.
    pub status: ProcessStatus,
    /// The captured stdout.
    pub stdout: Vec<u8>,
    /// The captured stderr.
    pub stderr: Vec<u8>,
}

impl Output {
    /// Get stdout as a string (lossy conversion).
    pub fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Get stderr as a string (lossy conversion).
    pub fn stderr_string(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// Error type for process operations.
#[derive(Debug)]
pub struct ProcessError {
    /// Error kind.
    pub kind: ProcessErrorKind,
    /// Error message.
    pub message: String,
    /// Underlying I/O error, if any.
    pub io_error: Option<io::Error>,
}

impl ProcessError {
    /// Create a new process error.
    pub fn new(kind: ProcessErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            io_error: None,
        }
    }

    /// Create from an I/O error.
    pub fn from_io(kind: ProcessErrorKind, error: io::Error) -> Self {
        Self {
            kind,
            message: error.to_string(),
            io_error: Some(error),
        }
    }
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ProcessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.io_error.as_ref().map(|e| e as &dyn std::error::Error)
    }
}

/// Kinds of process errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessErrorKind {
    /// Program not found.
    NotFound,
    /// Permission denied.
    PermissionDenied,
    /// Process was killed.
    Killed,
    /// I/O error.
    IoError,
    /// Invalid argument.
    InvalidArgument,
    /// Other error.
    Other,
}

// ============================================================================
// Convenience Functions
// ============================================================================

/// Execute a command and return its status.
///
/// This is a convenience function for simple command execution.
pub fn run<S: AsRef<OsStr>>(program: S, args: &[&str]) -> io::Result<ProcessStatus> {
    let status = Command::new(program).args(args).status()?;
    Ok(ProcessStatus::from_exit_status(status))
}

/// Execute a command and capture its output.
///
/// This is a convenience function for commands that produce output.
pub fn output<S: AsRef<OsStr>>(program: S, args: &[&str]) -> io::Result<Output> {
    Command::new(program).args(args).output()
}

/// Execute a shell command.
///
/// Uses `/bin/sh -c` on Unix and `cmd /C` on Windows.
pub fn shell(command: &str) -> io::Result<Output> {
    #[cfg(unix)]
    {
        Command::new("/bin/sh").args(&["-c", command]).output()
    }
    #[cfg(windows)]
    {
        Command::new("cmd").args(&["/C", command]).output()
    }
}

/// Check if a program exists and is executable.
pub fn which<S: AsRef<OsStr>>(program: S) -> Option<PathBuf> {
    let program = program.as_ref();

    // If it's an absolute path, check directly
    let path = Path::new(program);
    if path.is_absolute() {
        if path.is_file() {
            return Some(path.to_path_buf());
        }
        return None;
    }

    // Search PATH
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(program);
            if candidate.is_file() {
                return Some(candidate);
            }
            // On Windows, also check with .exe extension
            #[cfg(windows)]
            {
                let with_exe = candidate.with_extension("exe");
                if with_exe.is_file() {
                    return Some(with_exe);
                }
            }
        }
    }

    None
}

/// Get the current process ID.
pub fn current_pid() -> u32 {
    std::process::id()
}

/// Exit the current process with the given exit code.
pub fn exit(code: i32) -> ! {
    std::process::exit(code)
}

/// Abort the current process.
pub fn abort() -> ! {
    std::process::abort()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_builder() {
        let mut cmd = Command::new("echo");
        cmd.arg("hello").arg("world");
        assert_eq!(cmd.get_program(), "echo");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, vec!["hello", "world"]);
    }

    #[test]
    fn test_command_args() {
        let mut cmd = Command::new("test");
        cmd.args(&["-a", "-b", "-c"]);
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args.len(), 3);
    }

    #[test]
    fn test_command_env() {
        let mut cmd = Command::new("test");
        cmd.env("FOO", "bar").env("BAZ", "qux");
        // Environment is internal, just verify it doesn't panic
    }

    #[test]
    fn test_command_current_dir() {
        let mut cmd = Command::new("test");
        cmd.current_dir("/tmp");
        assert_eq!(cmd.get_current_dir(), Some(Path::new("/tmp")));
    }

    #[test]
    fn test_stdio_default() {
        assert_eq!(Stdio::default(), Stdio::Inherit);
    }

    #[test]
    fn test_process_status_success() {
        let status = ProcessStatus::from_code(0);
        assert!(status.success());
        assert_eq!(status.code(), Some(0));
    }

    #[test]
    fn test_process_status_failure() {
        let status = ProcessStatus::from_code(1);
        assert!(!status.success());
        assert_eq!(status.code(), Some(1));
    }

    #[test]
    fn test_process_status_display() {
        let status = ProcessStatus::from_code(42);
        assert!(format!("{}", status).contains("42"));
    }

    #[test]
    fn test_echo_output() {
        // Skip if echo is not available
        if which("echo").is_none() {
            return;
        }

        let output = Command::new("echo").arg("hello").output().unwrap();
        assert!(output.status.success());
        assert!(output.stdout_string().contains("hello"));
    }

    #[test]
    fn test_false_status() {
        // The 'false' command always returns exit code 1
        if which("false").is_none() {
            return;
        }

        let output = Command::new("false").output().unwrap();
        assert!(!output.status.success());
        assert_eq!(output.status.code(), Some(1));
    }

    #[test]
    fn test_which_echo() {
        // echo should exist on all Unix systems
        #[cfg(unix)]
        {
            let path = which("echo");
            assert!(path.is_some());
        }
    }

    #[test]
    fn test_which_nonexistent() {
        let path = which("this_program_definitely_does_not_exist_12345");
        assert!(path.is_none());
    }

    #[test]
    fn test_shell_command() {
        #[cfg(unix)]
        {
            let output = shell("echo hello").unwrap();
            assert!(output.status.success());
            assert!(output.stdout_string().contains("hello"));
        }
    }

    #[test]
    fn test_current_pid() {
        let pid = current_pid();
        assert!(pid > 0);
    }

    #[test]
    fn test_run_convenience() {
        if which("true").is_none() {
            return;
        }

        let status = run("true", &[]).unwrap();
        assert!(status.success());
    }

    #[test]
    fn test_output_convenience() {
        if which("echo").is_none() {
            return;
        }

        let output = output("echo", &["test"]).unwrap();
        assert!(output.status.success());
    }

    #[test]
    fn test_process_error() {
        let err = ProcessError::new(ProcessErrorKind::NotFound, "program not found");
        assert_eq!(err.kind, ProcessErrorKind::NotFound);
        assert!(err.to_string().contains("program not found"));
    }

    #[test]
    fn test_spawn_and_wait() {
        if which("true").is_none() {
            return;
        }

        let mut child = Command::new("true").spawn().unwrap();
        assert!(child.pid() > 0);
        let status = child.wait().unwrap();
        assert!(status.success());
    }

    #[test]
    fn test_spawn_with_piped_io() {
        if which("cat").is_none() {
            return;
        }

        let mut child = Command::new("cat")
            .stdin(Stdio::Piped)
            .stdout(Stdio::Piped)
            .spawn()
            .unwrap();

        // Write to stdin
        {
            let mut stdin = child.stdin().unwrap();
            stdin.write_all(b"hello").unwrap();
        }

        // Read from stdout
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"hello");
    }

    #[test]
    fn test_configured_compute_timeout() {
        // Without runtime config, should return default 30 seconds
        let timeout = configured_compute_timeout();
        assert_eq!(timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_wait_timeout_immediate_exit() {
        if which("true").is_none() {
            return;
        }

        let mut child = Command::new("true").spawn().unwrap();
        // 'true' exits immediately, so this should succeed
        let result = child.wait_timeout(Duration::from_secs(5));
        assert!(result.is_ok());
        let status = result.unwrap();
        assert!(status.is_some());
        assert!(status.unwrap().success());
    }

    #[test]
    fn test_wait_timeout_expires() {
        if which("sleep").is_none() {
            return;
        }

        let mut child = Command::new("sleep").arg("10").spawn().unwrap();
        // Wait with a very short timeout - should timeout
        let result = child.wait_timeout(Duration::from_millis(50));
        assert!(result.is_ok());
        let status = result.unwrap();
        // Should have timed out, so status is None
        assert!(status.is_none());
        // Kill the child to clean up
        let _ = child.kill();
    }

    #[test]
    fn test_wait_timeout_or_kill() {
        if which("sleep").is_none() {
            return;
        }

        let mut child = Command::new("sleep").arg("10").spawn().unwrap();
        // Wait with a short timeout - should kill the process
        let result = child.wait_timeout_or_kill(Duration::from_millis(100));
        assert!(result.is_ok());
        let status = result.unwrap();
        // Process was killed, so should not have exit code 0
        assert!(!status.success());
    }

    #[test]
    fn test_wait_with_configured_timeout() {
        if which("true").is_none() {
            return;
        }

        let mut child = Command::new("true").spawn().unwrap();
        // This should succeed with the default 30s timeout
        let result = child.wait_with_configured_timeout();
        assert!(result.is_ok());
        let status = result.unwrap();
        assert!(status.is_some());
        assert!(status.unwrap().success());
    }
}
