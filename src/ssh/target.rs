//! Validation for SSH destinations. The target is passed to ssh/scp as an argv
//! element, so anything that could be read as an option (`-oProxyCommand=...`)
//! or that carries shell-significant text is rejected before it gets there.

/// Longest destination accepted (DNS names are at most 253 characters).
const MAX_TARGET_LEN: usize = 255;

/// Check an SSH destination: `host`, `user@host`, `[ipv6]`, `user@[ipv6]`, or an
/// ssh config alias. Returns the target unchanged when it is safe to pass as argv.
pub fn validate_target(target: &str) -> Result<&str, String> {
    if target.is_empty() {
        return Err("SSH target is empty".into());
    }
    if target.len() > MAX_TARGET_LEN {
        return Err(format!("SSH target is longer than {MAX_TARGET_LEN} characters"));
    }
    if target.starts_with('-') {
        return Err("SSH target must not start with '-'".into());
    }
    if let Some(c) = target.chars().find(|c| !is_allowed_char(*c)) {
        return Err(format!("SSH target contains a disallowed character: {c:?}"));
    }
    if target.matches('@').count() > 1 {
        return Err("SSH target has more than one '@'".into());
    }

    let (user, host) = match target.split_once('@') {
        Some((user, host)) => (Some(user), host),
        None => (None, target),
    };
    if let Some(user) = user {
        if user.is_empty() {
            return Err("SSH target has an empty user name".into());
        }
        if user.chars().any(|c| matches!(c, '[' | ']' | ':' | '%')) {
            return Err("SSH target has an invalid user name".into());
        }
    }

    if host.is_empty() {
        return Err("SSH target has an empty host".into());
    }
    if host.starts_with('-') {
        return Err("SSH host must not start with '-'".into());
    }
    if let Some(inner) = host.strip_prefix('[') {
        let inner = inner
            .strip_suffix(']')
            .ok_or_else(|| "SSH target has an unclosed '['".to_string())?;
        if inner.is_empty()
            || !inner
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '%' | '-' | '_'))
        {
            return Err("SSH target has an invalid IPv6 address".into());
        }
    } else if host.chars().any(|c| matches!(c, '[' | ']' | ':')) {
        // ssh takes the port with -p, not host:port, and bare IPv6 needs brackets.
        return Err("SSH target must not contain ':' outside [ipv6]; use ~/.ssh/config for ports".into());
    }

    Ok(target)
}

fn is_allowed_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | ':' | '[' | ']' | '%' | '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_targets() {
        for t in [
            "host.example.com",
            "user@host.example.com",
            "dev-server_1",
            "root@10.0.0.5",
            "[::1]",
            "user@[2001:db8::1]",
            "user@[fe80::1%eth0]",
        ] {
            assert!(validate_target(t).is_ok(), "should accept {t:?}");
        }
    }

    #[test]
    fn rejects_malicious_targets() {
        for t in [
            "-oProxyCommand=touch /tmp/pwned", // option injection
            "--help",                          // option injection
            "host name",                       // whitespace splits argv
            "host\nRemoteCommand=id",          // control char / newline
            "$(id)",                           // shell substitution
            "host;id",                         // shell separator
            "host:2222",                       // port must not ride in the host
            "a@b@c",                           // ambiguous user/host split
            "user@-oProxyCommand=x",           // host starting with '-'
            "[::1",                            // unclosed bracket
            "user@",                           // empty host
            "",                                // empty
        ] {
            assert!(validate_target(t).is_err(), "should reject {t:?}");
        }
    }
}
