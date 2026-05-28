use directories::ProjectDirs;
use std::path::PathBuf;

fn project() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "paneldecontrol")
}

pub fn config_dir() -> Option<PathBuf> {
    project().map(|d| d.config_dir().to_path_buf())
}

pub fn state_path() -> Option<PathBuf> {
    project().map(|d| d.data_local_dir().join("state.toml"))
}
