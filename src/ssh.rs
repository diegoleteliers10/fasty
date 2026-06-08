use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SshHost {
    pub name: String,
    pub hostname: String,
    pub user: String,
    pub port: u16,
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
        ];
        if self.port != 22 {
            args.push("-p".into());
            args.push(self.port.to_string());
        }
        args.push(self.display());
        args
    }
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

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            if let Some(name) = current_name.take() {
                let hostname = current_hostname.unwrap_or_else(|| name.clone());
                hosts.push(SshHost {
                    name,
                    hostname,
                    user: current_user.clone(),
                    port: current_port,
                });
                current_hostname = None;
                current_user = String::from("root");
                current_port = 22;
            }
            continue;
        }

        let lower = trimmed.to_lowercase();
        let keyword = lower.split_whitespace().next().unwrap_or("");
        match keyword {
            "host" => {
                if let Some(name) = current_name.take() {
                    let hostname = current_hostname.unwrap_or_else(|| name.clone());
                    hosts.push(SshHost {
                        name,
                        hostname,
                        user: current_user.clone(),
                        port: current_port,
                    });
                    current_hostname = None;
                    current_user = String::from("root");
                    current_port = 22;
                }
                let rest = &trimmed[4..];
                let name = rest.trim().to_string();
                if !name.is_empty() && !name.contains('*') {
                    current_name = Some(name);
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
        hosts.push(SshHost {
            name,
            hostname,
            user: current_user,
            port: current_port,
        });
    }

    hosts.sort_by(|a, b| a.name.cmp(&b.name));
    hosts
}
