use std::fs;
use std::io;
use std::path::PathBuf;

use crate::paths;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub fleet_py: Option<PathBuf>,
    pub fleet_hub: Option<PathBuf>,
    pub agents: Vec<String>,
}

impl Config {
    pub fn load() -> Self {
        match fs::read_to_string(paths::config_path()) {
            Ok(content) => parse_config(&content),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        let path = paths::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_toml())
    }

    fn to_toml(&self) -> String {
        let mut out = String::new();
        if let Some(path) = &self.fleet_py {
            out.push_str(&format!("fleet_py = \"{}\"\n", escape(path.display())));
        }
        if let Some(path) = &self.fleet_hub {
            out.push_str(&format!("fleet_hub = \"{}\"\n", escape(path.display())));
        }
        if !self.agents.is_empty() {
            let agents = self
                .agents
                .iter()
                .map(|agent| format!("\"{}\"", escape(agent)))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("agents = [{agents}]\n"));
        }
        out
    }
}

pub fn parse_config(content: &str) -> Config {
    let mut config = Config::default();
    for raw_line in content.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "fleet_py" => config.fleet_py = parse_string(value).map(PathBuf::from),
            "fleet_hub" => config.fleet_hub = parse_string(value).map(PathBuf::from),
            "agents" => config.agents = parse_string_array(value),
            _ => {}
        }
    }
    config
}

fn parse_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return Some(unescape(&value[1..value.len() - 1]));
    }
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_string_array(value: &str) -> Vec<String> {
    let value = value.trim();
    let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
        return Vec::new();
    };
    inner
        .split(',')
        .filter_map(parse_string)
        .filter(|value| !value.is_empty())
        .collect()
}

fn escape(value: impl std::fmt::Display) -> String {
    value.to_string().replace('\\', "\\\\").replace('"', "\\\"")
}

fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::parse_config;

    #[test]
    fn parses_config_values() {
        let config = parse_config(
            r#"
fleet_py = "/tmp/fleet.py"
fleet_hub = "/tmp/hub"
agents = ["codex", "claude", "opencode"]
"#,
        );
        assert_eq!(config.fleet_py.unwrap().to_str().unwrap(), "/tmp/fleet.py");
        assert_eq!(config.fleet_hub.unwrap().to_str().unwrap(), "/tmp/hub");
        assert_eq!(config.agents, ["codex", "claude", "opencode"]);
    }
}
