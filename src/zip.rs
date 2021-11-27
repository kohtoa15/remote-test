use std::{error::Error, path::PathBuf};

use crate::hash::hash;

pub struct ZipFile {
    hash: Vec<u8>,
    path: PathBuf,
}

impl From<(Vec<u8>, PathBuf)> for ZipFile {
    fn from(tuple: (Vec<u8>, PathBuf)) -> Self {
        let (hash, path) = tuple;
        ZipFile { hash, path }
    }
}

impl ZipFile {
    /// Create local zip file from sent content
    pub async fn from_contents(content: Vec<u8>, base_dir: &PathBuf) -> Result<ZipFile, Box<dyn Error>> {
        // Generate path for local file
        let path = {
            let mut p = base_dir.clone();
            let timestamp = chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let filename = format!("zip-cached-{}", timestamp.as_str());
            p.push(filename);
            p
        };
        // Calculate hash from content
        let hash = hash(&content).await;
        // Write content to local path
        let _ = tokio::fs::write(path.as_path(), content).await?;
        // Create zipfile struct
        Ok(ZipFile::from((hash, path)))
    }

    /// Extract this zipfile into the target folder
    pub async fn extract_into(self, dir: &PathBuf) -> Result<(), Box<dyn Error>> {
        // Read zipfile content from file
        let zipfile = tokio::fs::File::open(self.path).await?;
        let mut zip = zip::ZipArchive::new(zipfile.try_into_std().unwrap())?;
        // Extract into target dir
        let _ = zip.extract(dir)?;
        Ok(())
    }

    pub fn compare_hash(&self, other: &[u8]) -> bool {
        self.hash.as_slice().eq(other)
    }
}
