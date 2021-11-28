use std::{error::Error, io::Cursor, path::{Path, PathBuf}};

use regex::Regex;
use walkdir::WalkDir;
use zip::ZipWriter;

use crate::hash::hash;

pub struct ZipFile {
    hash: String,
    path: PathBuf,
}

impl From<(String, PathBuf)> for ZipFile {
    fn from(tuple: (String, PathBuf)) -> Self {
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

    pub fn compare_hash(&self, other: &String) -> bool {
        self.hash == *other
    }

    pub fn get_hash(&self) -> &str {
        self.hash.as_str()
    }
}

pub struct ZipBlob {
    zip: ZipWriter<Cursor<Vec<u8>>>,
    options: zip::write::FileOptions,
    exclude: Vec<Regex>
}

impl ZipBlob {
    pub fn new(exclude: Vec<String>) -> Result<Self, Box<dyn Error>> {
        let exclude: Vec<Result<Regex, regex::Error>> = exclude.into_iter()
            .map(|e| Regex::new(e.as_str()))
            .collect();
        // Extract possible regex parse errors
        let err = exclude.iter()
            .find_map(|e| match e {
                Ok(_) => None,
                Err(e) => Some(e.to_owned()),
            });
        if let Some(err) = err {
            return Err(Box::new(err));
        }
        // Use parsed Regex patterns
        let exclude = exclude.into_iter().filter_map(|e| e.ok()).collect();

        // Create zip writer
        let zip = ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::FileOptions::default();

        Ok(ZipBlob {
            zip, options, exclude
        })
    }

    pub async fn add_dir(&mut self, dir: impl AsRef<Path>) -> Result<(), Box<dyn Error>> {
        use std::io::Write;
        println!("\n*** Compressing project directory");
        let walker = WalkDir::new(dir).min_depth(1);
        for entry in walker {
            let dir_entry = entry?;

            // Filter out excluded paths
            let path;
            if dir_entry.path_is_symlink() {
                path = std::fs::read_link(dir_entry.path())?;
            } else {
                path = dir_entry.path().to_path_buf();
            };
            let path_str = path.to_string_lossy();
            let is_excluded = self.exclude.iter().any(|p| p.is_match(path_str.as_ref()));
            // Filter out paths matching excluded regex
            if is_excluded {
                // Skip this entry
                continue;
            }

            // Feed to zip writer if file
            if dir_entry.file_type().is_file() {
                println!("\t{}", path_str);
                self.zip.start_file(path_str, self.options.clone())?;
                let content = tokio::fs::read(path.as_path()).await?;
                self.zip.write(&content)?;
            }
        }
        Ok(())
    }

    /// Finalizes zip process and returns a tuple of the base64-encoded hash
    /// and the actual data blob
    pub async fn finish(mut self) -> Result<(String, Vec<u8>), Box<dyn Error>> {
        let blob = self.zip.finish()?.into_inner();
        // Calculate hash for data blob
        let hash = hash(&blob).await;
        Ok((hash, blob))
    }
}
