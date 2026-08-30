use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SshHost {
    pub name: String,
    pub hostname: String,
    pub user: String,
    pub port: u16,
    pub tag: String,
}

impl SshHost {
    pub fn display(&self) -> String {
        if self.port == 22 {
            format!("{}@{}", self.user, self.hostname)
        } else {
            format!("{}@{}:{}", self.user, self.hostname, self.port)
        }
    }

    pub fn ssh_args(&self) -> Vec<String> {
        let mut args = vec![
            "-o".into(),
            "StrictHostKeyChecking=accept-new".into(),
            "-o".into(),
            "ServerAliveInterval=15".into(),
            "-o".into(),
            "ServerAliveCountMax=3".into(),
            "-o".into(),
            "TCPKeepAlive=yes".into(),
        ];
        if self.port != 22 {
            args.push("-p".into());
            args.push(self.port.to_string());
        }
        args.push(self.display());
        args
    }

    pub fn resilient_shell_command(&self) -> (String, Vec<String>) {
        let target = self.display();
        let port_arg = if self.port != 22 { format!("-p {}", self.port) } else { String::new() };
        let cmd = format!(
            r#"while true; do ssh -o StrictHostKeyChecking=accept-new -o ServerAliveInterval=15 -o ServerAliveCountMax=3 -o TCPKeepAlive=yes {} {}; status=$?; if [ $status -eq 0 ]; then break; fi; printf "\n\033[1;33m[fastty mux] Connection dropped (exit code %d).\033[0m\n\033[1;36mPress [r] to reconnect or any key to exit...\033[0m " "$status"; read -n 1 -r key; echo ""; if [ "$key" != "r" ] && [ "$key" != "R" ]; then break; fi; printf "\033[2J\033[H\033[1;32m[fastty mux] Reconnecting to {}...\033[0m\n"; sleep 1; done"#,
            port_arg, target, target
        );
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        (shell, vec!["-c".into(), cmd])
    }
}

fn deduce_tag(name: &str, hostname: &str) -> String {
    let lower_name = name.to_lowercase();
    let lower_host = hostname.to_lowercase();
    let combined = format!("{lower_name} {lower_host}");

    if combined.contains("prod") || combined.contains("production") {
        "prod".to_string()
    } else if combined.contains("stage") || combined.contains("staging") {
        "staging".to_string()
    } else if combined.contains("dev") || combined.contains("develop") || combined.contains("local") {
        "dev".to_string()
    } else if combined.contains("aws") || combined.contains("amazon") || combined.contains("ec2") {
        "aws".to_string()
    } else if combined.contains("gcp") || combined.contains("google") {
        "gcp".to_string()
    } else if combined.contains("hetzner") || combined.contains("vps") || combined.contains("do") || combined.contains("digitalocean") {
        "vps".to_string()
    } else if combined.contains("home") || combined.contains("nas") || combined.contains("pi") || combined.contains("lab") {
        "homelab".to_string()
    } else {
        "servers".to_string()
    }
}

pub fn get_all_tags(hosts: &[SshHost]) -> Vec<String> {
    let mut tags = Vec::new();
    for h in hosts {
        if !tags.contains(&h.tag) {
            tags.push(h.tag.clone());
        }
    }
    tags.sort();
    tags
}

pub fn parse_ssh_config() -> Vec<SshHost> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    let path = PathBuf::from(home).join(".ssh/config");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut hosts = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_hostname: Option<String> = None;
    let mut current_user = String::from("root");
    let mut current_port: u16 = 22;
    let mut current_tag: Option<String> = None;
    let mut pending_comment_tag: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with('#') {
            let comment = trimmed.trim_start_matches('#').trim();
            if let Some(t) = comment.strip_prefix("tag:").or_else(|| comment.strip_prefix("group:")) {
                pending_comment_tag = Some(t.trim().to_string());
            }
            continue;
        }

        let lower = trimmed.to_lowercase();
        let keyword = lower.split_whitespace().next().unwrap_or("");
        match keyword {
            "host" => {
                if let Some(name) = current_name.take() {
                    let hostname = current_hostname.unwrap_or_else(|| name.clone());
                    let tag = current_tag.take().unwrap_or_else(|| deduce_tag(&name, &hostname));
                    hosts.push(SshHost {
                        name,
                        hostname,
                        user: current_user.clone(),
                        port: current_port,
                        tag,
                    });
                    current_hostname = None;
                    current_user = String::from("root");
                    current_port = 22;
                }
                let rest = &trimmed[4..];
                let name = rest.trim().to_string();
                if !name.is_empty() && !name.contains('*') {
                    current_name = Some(name);
                    current_tag = pending_comment_tag.take();
                }
                continue;
            }
            "hostname" => {
                if current_name.is_some() {
                    let rest = &lower[8..];
                    let val = rest.trim().strip_prefix('=').or_else(|| rest.split_whitespace().nth(1));
                    if let Some(v) = val {
                        current_hostname = Some(v.trim().to_string());
                    }
                }
                continue;
            }
            "user" => {
                if current_name.is_some() {
                    let rest = &lower[4..];
                    let val = rest.trim().strip_prefix('=').or_else(|| rest.split_whitespace().nth(1));
                    if let Some(v) = val {
                        current_user = v.trim().to_string();
                    }
                }
                continue;
            }
            "port" => {
                if current_name.is_some() {
                    let rest = &lower[4..];
                    let val = rest.trim().strip_prefix('=').or_else(|| rest.split_whitespace().nth(1));
                    if let Some(v) = val {
                        if let Ok(p) = v.trim().parse::<u16>() {
                            current_port = p;
                        }
                    }
                }
                continue;
            }
            _ => {}
        }
    }

    if let Some(name) = current_name {
        let hostname = current_hostname.unwrap_or_else(|| name.clone());
        let tag = current_tag.unwrap_or_else(|| deduce_tag(&name, &hostname));
        hosts.push(SshHost {
            name,
            hostname,
            user: current_user,
            port: current_port,
            tag,
        });
    }

    hosts.sort_by(|a, b| a.name.cmp(&b.name));
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deduce_tag() {
        assert_eq!(deduce_tag("prod-db", "10.0.1.5"), "prod");
        assert_eq!(deduce_tag("web-dev", "localhost"), "dev");
        assert_eq!(deduce_tag("my-box", "ec2-54.aws.com"), "aws");
        assert_eq!(deduce_tag("pi-hole", "192.168.1.50"), "homelab");
    }

    #[test]
    fn test_ssh_args_and_resilient_command() {
        let host = SshHost {
            name: "prod-server".into(),
            hostname: "prod.example.com".into(),
            user: "ubuntu".into(),
            port: 2222,
            tag: "prod".into(),
        };
        let args = host.ssh_args();
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"2222".to_string()));
        assert!(args.contains(&"ubuntu@prod.example.com:2222".to_string()));

        let (shell, cmd_args) = host.resilient_shell_command();
        assert!(!shell.is_empty());
        assert_eq!(cmd_args[0], "-c");
        assert!(cmd_args[1].contains("StrictHostKeyChecking=accept-new"));
    }
}
