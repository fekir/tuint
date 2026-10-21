use std::{
    io::ErrorKind,
    process::Command,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::common::process::ProcessInfo;

fn decode_output(data: &[u8]) -> String {
    if data.len() >= 2 {
        let utf16 = data
            .chunks_exact(2)
            .map(|x| u16::from_le_bytes([x[0], x[1]]))
            .collect::<Vec<_>>();
        let decoded = String::from_utf16_lossy(&utf16);
        if !decoded.contains('\0') && !decoded.trim().is_empty() {
            return decoded;
        }
    }
    String::from_utf8_lossy(data).to_string()
}

pub(crate) fn get_distributions() -> Result<Vec<String>, String> {
    let output = match std::process::Command::new("wsl.exe")
        .args(["--list", "--running", "--quiet"])
        .output()
    {
        Ok(output) => output,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(e) => {
            return Err(format!("Failed to execute wsl.exe: {e}"));
        }
    };

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }

    let text = decode_output(&output.stdout);

    let distributions = text
        .lines()
        .map(|line| line.trim().trim_matches('\0'))
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(distributions)
}

fn parse_process(line: &str, now: u64) -> Option<ProcessInfo> {
    let mut parts = line.split_whitespace();

    let pid = parts.next()?.parse().ok()?;
    let ppid = parts.next()?.parse().ok()?;
    let cpu = parts.next()?.parse().unwrap_or(0.0);
    let memory = parts.next()?.parse().unwrap_or(0.0);
    let etimes = parts.next()?.parse().unwrap_or(0.0);

    let command = parts.collect::<Vec<_>>().join(" ");

    Some(ProcessInfo {
        pid: pid,
        ppid: ppid,
        name: command,
        cpu_usage_percent: cpu,
        memory_usage_percent: memory,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::parse_process;

    #[test]
    fn parse_process_test() {
        let process = parse_process("123 1 17.5 3.2 30 /usr/bin/example --parameter", 100).unwrap();
        assert_eq!(process.pid, 123);
        assert_eq!(process.ppid, 1);
        assert_eq!(process.cpu_usage_percent, 17.5);
        assert_eq!(process.memory_usage_percent, 3.2);
        assert_eq!(process.name, "/usr/bin/example --parameter");
    }
}

fn prune_empty_ancestors(processes: &mut Vec<ProcessInfo>, start_parent: u32) {
    let mut current = start_parent;

    while let Some(process) = processes.iter().find(|process| process.pid == current) {
        let next = process.ppid;
        let has_children = processes.iter().any(|p| p.ppid == current);
        if has_children {
            break;
        }
        processes.retain(|p| p.pid != current);
        current = next;
    }
}

pub(crate) fn get_processes(distro: &str) -> Result<Vec<ProcessInfo>, String> {
    // ,comm= -> name, no args, but trimmed
    // ,args= -> full path and args
    // ?? -    > full path, no args
    const PS_FORMAT: &str = "pid=,ppid=,pcpu=,pmem=,etimes=,args=";

    // run with root, does not require admin priv on windows
    // note: some wsl instances might not have root (fallback to default user) or ps (currently no fallback)
    let output = Command::new("wsl.exe")
        .args([
            "-d", distro, "--user", "root", "--exec", "ps", "-eo", PS_FORMAT,
        ])
        .output()
        .map_err(|e| format!("Failed to execute wsl.exe: {e}"))?;

    let output = if output.status.success() {
        output
    } else {
        Command::new("wsl.exe")
            .args(["-d", distro, "--exec", "ps", "-eo", PS_FORMAT])
            .output()
            .map_err(|e| format!("Failed to execute wsl.exe: {e}"))?
    };

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }

    let text = String::from_utf8_lossy(&output.stdout);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut processes: Vec<ProcessInfo> = text
        .lines()
        .filter_map(|line| parse_process(line, now))
        .collect();

    // exclude ps, it's temporary, but might create a dixtracting tree
    let ps_command_prefix = format!("ps -eo {PS_FORMAT}");
    let ps_parents: Vec<u32> = processes
        .iter()
        .filter(|process| process.name.starts_with(&ps_command_prefix))
        .map(|process| process.ppid)
        .collect();

    processes.retain(|process| !process.name.starts_with(&ps_command_prefix));

    for parent in ps_parents {
        prune_empty_ancestors(&mut processes, parent);
    }

    let subsystem: Rc<str> = Rc::from(distro);
    for process in &mut processes {
        process.subsystem = Rc::clone(&subsystem);
    }

    Ok(processes)
}

pub(crate) fn kill_wsl_process(pi: &ProcessInfo) -> std::io::Result<()> {
    // fixme:fallback current user if no root
    let status = Command::new("wsl.exe")
        .args([
            "--distribution",
            &pi.subsystem,
            "--user",
            "root",
            "--exec",
            "kill",
            //"-TERM",
            "-s",
            "9",
        ])
        .arg(pi.pid.to_string())
        .status()?;

    if !status.success() {
        return Err(std::io::Error::other(format!(
            "wsl kill exited with {status}"
        )));
    }

    Ok(())
}
