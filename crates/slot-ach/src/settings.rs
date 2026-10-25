//! `Config/achievements.txt`: whether achievements are on, and the account. Only the login
//! token is kept, never the password.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub user: String,
    pub token: String,
}

pub const FILE: &str = "achievements.txt";

impl Settings {
    pub fn path(root: &Path) -> PathBuf {
        root.join("Config").join(FILE)
    }

    pub fn load(root: &Path) -> Settings {
        let text = std::fs::read_to_string(Self::path(root)).unwrap_or_default();
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "enabled" => s.enabled = matches!(v, "on" | "true" | "yes" | "1"),
                "user" => s.user = v.to_string(),
                "token" => s.token = v.to_string(),
                _ => {}
            }
        }
        s
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let path = Self::path(root);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = format!(
            "# RetroAchievements. Safe to edit while slot is off.\n\
             # The token comes from logging in on the device; the password is not kept.\n\
             enabled={}\nuser={}\ntoken={}\n",
            if self.enabled { "on" } else { "off" },
            self.user,
            self.token
        );
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)
    }
}
