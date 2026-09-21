//! Submit URLs and local documents to the desktop's registered application.
use anyhow::{Context, Result, bail, ensure};
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

pub fn execute(project: &Path, target: &str, parameters: &str) -> Result<bool> {
    // xdg-open/open do not implement Windows executable command-line syntax.
    ensure!(
        parameters.is_empty(),
        "executable launch parameters are not supported by the desktop opener"
    );
    let target = normalize_target(project, target)?;
    let program = if cfg!(target_os = "linux") || cfg!(target_os = "freebsd") {
        "xdg-open"
    } else if cfg!(target_os = "macos") {
        "/usr/bin/open"
    } else {
        bail!("desktop shell execution is unavailable on this platform");
    };
    launch(program.as_ref(), project, &target)
}

fn normalize_target(project: &Path, target: &str) -> Result<String> {
    let target = target.trim();
    let target = target
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(target);
    ensure!(
        !target.is_empty() && !target.contains('\0'),
        "empty or invalid shell target"
    );
    if !target.starts_with("file://.") && is_uri(target) {
        return Ok(target.into());
    }
    let local = krkrz_core::local_storage_path(target);
    let path = project
        .join(local)
        .canonicalize()
        .context("resolve shell target")?;
    Ok(path.to_string_lossy().into_owned())
}

fn is_uri(target: &str) -> bool {
    target.split_once(':').is_some_and(|(scheme, _)| {
        scheme.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && scheme.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
            // A Windows drive path is not a URI scheme.
            && !(scheme.len() == 1 && target.as_bytes().get(2).is_some_and(|c| matches!(c, b'/' | b'\\')))
    })
}

fn launch(program: &Path, project: &Path, target: &str) -> Result<bool> {
    // Pass one literal argument. A URL's &, quotes, or $() are never shell code.
    let mut child = Command::new(program)
        .arg(target)
        .current_dir(project)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start desktop opener")?;
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = child.wait();
        if !result.as_ref().is_ok_and(|status| status.success()) {
            eprintln!("System.shellExecute: desktop opener failed: {result:?}");
        }
        let _ = tx.send(result);
    });
    // Some desktop handlers keep the opener alive until their window closes.
    // Report immediate failure, but do not hold the game for that lifetime.
    match rx.recv_timeout(Duration::from_millis(200)) {
        Ok(status) => Ok(status?.success()),
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(true),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_stay_literal_and_documents_resolve_from_the_project() {
        let dir = tempfile::tempdir().unwrap();
        for url in [
            "https://example.invalid/a?q=x&b=$(touch%20bad)",
            "mailto:test@example.invalid",
            "file:///tmp/a%20b.txt",
        ] {
            assert_eq!(normalize_target(dir.path(), url).unwrap(), url);
        }
        let path = dir.path().join("説明 file.txt");
        std::fs::write(&path, "text").unwrap();
        for target in [
            "説明 file.txt".to_owned(),
            format!("\"{}\"", path.display()),
            format!("file://.{}", path.display()),
        ] {
            assert_eq!(
                normalize_target(dir.path(), &target).unwrap(),
                path.to_string_lossy()
            );
        }
        assert!(!is_uri("C:\\game\\readme.txt"));
        for target in ["", "\"\"", "missing.txt", "https://example.invalid/\0"] {
            assert!(normalize_target(dir.path(), target).is_err());
        }
        assert!(execute(dir.path(), "https://example.invalid/", "--argument").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn opener_receives_one_argument_and_reports_failure_without_running_shell_text() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let opener = dir.path().join("fake-opener");
        std::fs::write(
            &opener,
            "#!/bin/sh\nprintf '%s\\n' \"$#\" \"$1\" > arguments\npwd > cwd\n",
        )
        .unwrap();
        std::fs::set_permissions(&opener, std::fs::Permissions::from_mode(0o700)).unwrap();
        let target = "https://example.invalid/a?one=1&two=$(touch marker);\"quoted\"";
        assert!(launch(&opener, dir.path(), target).unwrap());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("arguments")).unwrap(),
            format!("1\n{target}\n")
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("cwd"))
                .unwrap()
                .trim(),
            dir.path().to_str().unwrap()
        );
        assert!(!dir.path().join("marker").exists());
        std::fs::write(&opener, "#!/bin/sh\nexit 3\n").unwrap();
        assert!(!launch(&opener, dir.path(), target).unwrap());
        assert!(launch(&dir.path().join("absent"), dir.path(), target).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_foreground_opener_does_not_hold_the_game_until_its_window_closes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let opener = dir.path().join("foreground-opener");
        // A FIFO models a desktop handler that remains alive. Release it after
        // launch returns, so a blocking implementation cannot pass this test.
        assert!(
            Command::new("mkfifo")
                .arg(dir.path().join("close"))
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(
            &opener,
            "#!/bin/sh\nread reply < close\nprintf done > finished\n",
        )
        .unwrap();
        std::fs::set_permissions(&opener, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = dir.path().to_owned();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = tx.send(launch(&opener, &root, "file.txt"));
        });
        let result = rx.recv_timeout(Duration::from_secs(3));
        // Always release the child, even when the bounded-launch assertion fails.
        std::fs::write(dir.path().join("close"), "close\n").unwrap();
        worker.join().unwrap();
        assert!(result.unwrap().unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !dir.path().join("finished").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(dir.path().join("finished").exists());
    }
}
