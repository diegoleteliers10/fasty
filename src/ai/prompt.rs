use std::path::Path;

pub fn system_prompt(cwd: &Path) -> String {
    let os = std::env::consts::OS;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());

    let git_info = if let Ok(repo) = git2::Repository::discover(cwd) {
        let head = repo.head().ok();
        let branch = head
            .as_ref()
            .and_then(|h| h.shorthand().ok())
            .unwrap_or("detached");
        format!(" (git branch: {})", branch)
    } else {
        String::new()
    };

    format!(
        "You are Fastty AI, an intelligent agent integrated directly into the Fastty GPU terminal.\n\
        Environment:\n\
        - OS: {}\n\
        - Shell: {}\n\
        - CWD: {}{}\n\n\
        Instructions:\n\
        - Use provided tools (read_file, edit_file, search, run_command) to inspect the codebase and perform actions.\n\
        - When running commands, avoid long-running blocking commands without backgrounding.\n\
        - Prefer small, deterministic steps and verify changes.\n\
        - Be extremely concise in your responses. Output explanations clearly without conversational filler.",
        os,
        shell,
        cwd.display(),
        git_info
    )
}
