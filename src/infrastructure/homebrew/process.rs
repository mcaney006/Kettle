use super::super::InfrastructureError;
use std::{
    collections::HashMap,
    ffi::OsString,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessEvent {
    pub stream: ProcessStream,
    pub message: String,
}

#[derive(Debug)]
pub(crate) struct ProcessOutput {
    pub stdout: String,
    pub stderr: String,
}

pub(crate) struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: HashMap<OsString, OsString>,
    pub current_dir: PathBuf,
}

pub(crate) fn run(
    spec: &CommandSpec,
    cancelled: &dyn Fn() -> bool,
    mut on_event: impl FnMut(ProcessEvent),
) -> Result<ProcessOutput, InfrastructureError> {
    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .envs(&spec.env)
        .current_dir(&spec.current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| InfrastructureError::Invocation {
            program: spec.program.clone(),
            source,
        })?;

    let (sender, receiver) = mpsc::channel();
    let stdout = child.stdout.take().ok_or_else(|| {
        InfrastructureError::PipeReader(std::io::Error::other("stdout pipe was not created"))
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        InfrastructureError::PipeReader(std::io::Error::other("stderr pipe was not created"))
    })?;
    let stdout_reader = spawn_reader(stdout, ProcessStream::Stdout, sender.clone());
    let stderr_reader = spawn_reader(stderr, ProcessStream::Stderr, sender);
    let mut captured_stdout = Vec::new();
    let mut captured_stderr = Vec::new();
    let mut killed = false;

    loop {
        if cancelled() && !killed {
            child
                .kill()
                .map_err(|source| InfrastructureError::ProcessWait {
                    program: spec.program.clone(),
                    source,
                })?;
            killed = true;
        }
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(event) => {
                match event.stream {
                    ProcessStream::Stdout => &mut captured_stdout,
                    ProcessStream::Stderr => &mut captured_stderr,
                }
                .push(event.message.clone());
                on_event(event);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if child
                    .try_wait()
                    .map_err(|source| InfrastructureError::ProcessWait {
                        program: spec.program.clone(),
                        source,
                    })?
                    .is_some()
                    && stdout_reader.is_finished()
                    && stderr_reader.is_finished()
                {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    join_reader(stdout_reader)?;
    join_reader(stderr_reader)?;
    let status = child
        .wait()
        .map_err(|source| InfrastructureError::ProcessWait {
            program: spec.program.clone(),
            source,
        })?;
    let output = ProcessOutput {
        stdout: captured_stdout.join("\n"),
        stderr: captured_stderr.join("\n"),
    };
    if cancelled() {
        return Err(InfrastructureError::Cancelled);
    }
    if status.success() {
        Ok(output)
    } else {
        Err(InfrastructureError::NonZeroExit {
            program: spec.program.clone(),
            code: status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

fn spawn_reader(
    pipe: impl std::io::Read + Send + 'static,
    stream: ProcessStream,
    sender: mpsc::Sender<ProcessEvent>,
) -> std::thread::JoinHandle<std::io::Result<()>> {
    std::thread::spawn(move || {
        for message in BufReader::new(pipe).lines() {
            let message = message?;
            if sender.send(ProcessEvent { stream, message }).is_err() {
                break;
            }
        }
        Ok(())
    })
}

fn join_reader(
    reader: std::thread::JoinHandle<std::io::Result<()>>,
) -> Result<(), InfrastructureError> {
    reader
        .join()
        .map_err(|_| InfrastructureError::PipeReader(std::io::Error::other("reader panicked")))?
        .map_err(InfrastructureError::PipeReader)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(script: &str) -> CommandSpec {
        CommandSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec![OsString::from("-c"), OsString::from(script)],
            env: HashMap::new(),
            current_dir: PathBuf::from("/"),
        }
    }

    #[test]
    fn exit_status_is_authoritative_and_streams_stay_distinct() {
        let mut events = Vec::new();
        let output = run(
            &shell("echo normal; echo warning >&2; exit 0"),
            &|| false,
            |event| events.push(event),
        )
        .unwrap();
        assert_eq!(output.stdout, "normal");
        assert_eq!(output.stderr, "warning");
        assert!(
            events
                .iter()
                .any(|event| event.stream == ProcessStream::Stdout)
        );
        assert!(
            events
                .iter()
                .any(|event| event.stream == ProcessStream::Stderr)
        );

        let error = run(&shell("echo normal; exit 3"), &|| false, |_| {}).unwrap_err();
        assert!(matches!(
            error,
            InfrastructureError::NonZeroExit { code: Some(3), .. }
        ));
    }

    #[test]
    fn mixed_verbose_output_cannot_deadlock() {
        let output = run(
            &shell("i=0; while [ $i -lt 1000 ]; do echo out-$i; echo err-$i >&2; i=$((i+1)); done"),
            &|| false,
            |_| {},
        )
        .unwrap();
        assert_eq!(output.stdout.lines().count(), 1000);
        assert_eq!(output.stderr.lines().count(), 1000);
    }

    #[test]
    fn cancellation_terminates_the_direct_child() {
        let started = std::time::Instant::now();
        let spec = CommandSpec {
            program: PathBuf::from("/bin/sleep"),
            args: vec![OsString::from("5")],
            env: HashMap::new(),
            current_dir: PathBuf::from("/"),
        };
        let error = run(
            &spec,
            &|| started.elapsed() >= Duration::from_millis(50),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(error, InfrastructureError::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
