use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Request {
    pub name: String,
    pub collection: String,
    pub method: String,
    pub url: String,
    pub params: String,
    pub headers: String,
    pub body: String,
    pub bearer: String,
}
impl Default for Request {
    fn default() -> Self {
        Self {
            name: "New request".into(),
            collection: "My collection".into(),
            method: "GET".into(),
            url: "{{base_url}}/get".into(),
            params: String::new(),
            headers: "accept: */*\nContent-Type: application/json".into(),
            body: String::new(),
            bearer: String::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct Environment(pub BTreeMap<String, String>);
impl Default for Environment {
    fn default() -> Self {
        Self(BTreeMap::from([(
            "base_url".into(),
            "http://localhost:8000".into(),
        )]))
    }
}
impl Environment {
    pub fn resolve(&self, input: &str) -> Result<String> {
        resolve(input, &|key| {
            self.0.get(key).cloned().or_else(|| std::env::var(key).ok())
        })
    }
}
// One pass intentionally keeps braces inside substituted values literal.
fn resolve(input: &str, lookup: &impl Fn(&str) -> Option<String>) -> Result<String> {
    let mut output = String::new();
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        let end = rest
            .find("}}")
            .context("Unclosed environment variable (expected }})")?;
        let key = rest[..end].trim();
        let value =
            lookup(key).with_context(|| format!("Undefined environment variable: {key}"))?;
        output.push_str(&value);
        rest = &rest[end + 2..];
    }
    output.push_str(rest);
    Ok(output)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub request: Request,
    pub outcome: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub version: u32,
    pub name: String,
    pub collections: Vec<String>,
    pub requests: Vec<Request>,
    pub environment: Environment,
    pub history: Vec<HistoryEntry>,
}
impl Default for Workspace {
    fn default() -> Self {
        Self {
            version: 2,
            name: "My workspace".into(),
            collections: vec!["My collection".into()],
            requests: vec![],
            environment: Environment::default(),
            history: vec![],
        }
    }
}
impl Workspace {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).context("Cannot open workspace file")?;
        let mut data: serde_json::Value = serde_json::from_slice(&bytes)
            .context("Invalid workspace JSON; original file has not been changed")?;
        match data.get("version").and_then(|v| v.as_u64()).unwrap_or(1) {
            1 => {
                let legacy = data
                    .get("environments")
                    .context("Missing legacy environment")?;
                let active = legacy
                    .get("active")
                    .and_then(|v| v.as_str())
                    .context("Missing active environment")?;
                let env = legacy
                    .get("profiles")
                    .and_then(|p| p.get(active))
                    .context("Missing active environment profile")?
                    .clone();
                data["environment"] = env;
                data["collections"] = serde_json::json!([]);
                data["version"] = 2.into();
                data["name"] = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
                    .into();
            }
            2 => {}
            version => bail!("Unsupported workspace version: {version}"),
        }
        let mut workspace: Self =
            serde_json::from_value(data).context("Invalid workspace structure")?;
        workspace.normalize()?;
        Ok(workspace)
    }
    pub fn normalize(&mut self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("Workspace name cannot be empty");
        }
        if self
            .requests
            .iter()
            .any(|r| !METHODS.contains(&r.method.as_str()))
        {
            bail!("Workspace contains an unsupported HTTP method");
        }
        for request in &mut self.requests {
            request.collection = request.collection.trim().to_string();
        }
        let mut collections = Vec::new();
        for name in self
            .collections
            .iter()
            .chain(self.requests.iter().map(|r| &r.collection))
        {
            let name = name.trim();
            if name.is_empty() {
                bail!("Collection name cannot be empty");
            }
            if !collections.iter().any(|n| n == name) {
                collections.push(name.to_string());
            }
        }
        self.collections = collections;
        self.history.truncate(50);
        Ok(())
    }
    pub fn add_collection(&mut self, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            bail!("Enter a collection name");
        }
        if self.collections.iter().any(|n| n == name) {
            bail!("A collection with this name already exists");
        }
        self.collections.push(name.to_string());
        Ok(())
    }
    pub fn rename_collection(&mut self, old: &str, new: &str) -> Result<()> {
        let new = new.trim();
        if new == old {
            return Ok(());
        }
        if new.is_empty() || self.collections.iter().any(|n| n == new) {
            bail!("Choose a nonempty, unique collection name");
        }
        let name = self
            .collections
            .iter_mut()
            .find(|n| n.as_str() == old)
            .context("Collection no longer exists")?;
        *name = new.to_string();
        for request in &mut self.requests {
            if request.collection == old {
                request.collection = new.to_string();
            }
        }
        Ok(())
    }
    pub fn delete_collection(&mut self, name: &str) {
        self.collections.retain(|n| n != name);
        self.requests.retain(|r| r.collection != name);
    }
    pub fn create(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        private_write(path, &serde_json::to_vec_pretty(self)?)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        if let Ok(bytes) = fs::read(path) {
            let old: serde_json::Value = serde_json::from_slice(&bytes)
                .context("Existing workspace is invalid; refusing to replace it")?;
            if old.get("version").and_then(|v| v.as_u64()).unwrap_or(1) == 1 {
                let backup = path.with_extension("v1-backup.json");
                if !backup.exists() {
                    private_write(&backup, &bytes).context("Cannot back up legacy workspace")?;
                }
            }
        }
        let temporary = path.with_extension("json.tmp");
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .context("Cannot create workspace temporary file")?;
        let result = (|| -> Result<()> {
            file.write_all(&serde_json::to_vec_pretty(self)?)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.context("Could not save workspace")
    }
}
pub fn config_dir() -> Result<PathBuf> {
    Ok(directories::ProjectDirs::from("", "", "postrust")
        .context("No config directory")?
        .config_dir()
        .to_path_buf())
}
pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .context("Cannot create file; choose an unused path")?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("postrust-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
    #[test]
    fn variables_are_strict_single_pass_and_unicode_safe() {
        let lookup = |key: &str| match key {
            "host" => Some("世界".into()),
            "secret" => Some("{{literal}}".into()),
            _ => None,
        };
        assert_eq!(
            resolve("https://{{ host }}/{{secret}}", &lookup).unwrap(),
            "https://世界/{{literal}}"
        );
        assert!(resolve("{{missing}}", &lookup).is_err());
        assert!(resolve("{{host", &lookup).is_err());
    }
    #[test]
    fn workspace_variables_override_process_environment() {
        let mut env = Environment::default();
        env.0.insert("PATH".into(), "workspace".into());
        assert_eq!(env.resolve("{{PATH}}").unwrap(), "workspace");
    }
    #[test]
    fn workspaces_keep_environments_separate_and_create_never_overwrites() {
        let dir = directory("isolation");
        let a = dir.join("a.json");
        let b = dir.join("b.json");
        let mut first = Workspace::default();
        first.environment.0.insert("token".into(), "first".into());
        first.create(&a).unwrap();
        let mut second = Workspace::default();
        second.environment.0.insert("token".into(), "second".into());
        second.create(&b).unwrap();
        assert!(second.create(&a).is_err());
        assert_eq!(
            Workspace::load(&a)
                .unwrap()
                .environment
                .resolve("{{token}}")
                .unwrap(),
            "first"
        );
        assert_eq!(
            Workspace::load(&b)
                .unwrap()
                .environment
                .resolve("{{token}}")
                .unwrap(),
            "second"
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn migration_preserves_active_environment_and_backs_up_original_profiles() {
        let dir = directory("migration");
        let path = dir.join("legacy.json");
        let source = r#"{"version":1,"requests":[{"collection":"API","name":"Echo"}],"environments":{"active":"Staging","profiles":{"Local":{"token":"local"},"Staging":{"token":"staging"}}}}"#;
        fs::write(&path, source).unwrap();
        let workspace = Workspace::load(&path).unwrap();
        assert_eq!(
            workspace.environment.resolve("{{token}}").unwrap(),
            "staging"
        );
        assert!(workspace.collections.contains(&"API".into()));
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        workspace.save(&path).unwrap();
        assert_eq!(
            fs::read_to_string(path.with_extension("v1-backup.json")).unwrap(),
            source
        );
        assert_eq!(Workspace::load(&path).unwrap().version, 2);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn collection_management_handles_empty_folders_and_moves_by_name() {
        let mut workspace = Workspace::default();
        workspace.add_collection("Empty").unwrap();
        assert!(workspace.add_collection(" Empty ").is_err());
        workspace.requests.push(Request {
            collection: "Empty".into(),
            ..Request::default()
        });
        workspace.rename_collection("Empty", "Renamed").unwrap();
        assert_eq!(workspace.requests[0].collection, "Renamed");
        workspace.delete_collection("Renamed");
        assert!(workspace.requests.is_empty());
    }
    #[test]
    fn workspace_roundtrip_is_private_and_invalid_files_are_preserved() {
        let dir = directory("roundtrip");
        let path = dir.join("workspace.json");
        let mut workspace = Workspace::default();
        workspace.requests.push(Request {
            body: "你好\n🌍".into(),
            ..Request::default()
        });
        workspace.save(&path).unwrap();
        assert_eq!(Workspace::load(&path).unwrap().requests, workspace.requests);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::write(&path, "invalid").unwrap();
        assert!(Workspace::load(&path).is_err());
        assert!(workspace.save(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "invalid");
        fs::remove_dir_all(dir).unwrap();
    }
}
