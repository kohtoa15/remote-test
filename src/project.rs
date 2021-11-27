use std::{error::Error, path::PathBuf, process::Stdio};

use serde::{Serialize, Deserialize};
use tokio::process::Command;

use crate::zip::ZipFile;
use crate::pb::TestResult;

pub type TestOutput = (String, Option<i32>, Vec<u8>, Vec<u8>);

impl From<TestOutput> for TestResult {
    fn from(t: TestOutput) -> Self {
        let (cmd, code, stdout, stderr) = t;
        // Report success if we have an exit code 0
        let success = code
            .filter(|x| *x == 0)
            .is_some();
        TestResult {
            command: cmd,
            stdout,
            stderr,
            success,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct TestProject {
    name: String,
    tests: Vec<Vec<String>>,
    hash: Option<Vec<u8>>,
}

impl TestProject {
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// Returns name/hash tuple of this project
    pub fn get_tuple(&self) -> (String, String) {
        let name = self.name.clone();
        let bytes = self.hash.clone()
            .unwrap_or_default();
        let hash = base64::encode_config(bytes, base64::STANDARD);
        (name, hash)
    }

    /// Use supplied data to apply update
    /// checks whether update can be applied before and returns Ok(false) if no
    /// update can be applied
    pub async fn apply_update(&mut self, content: ZipFile, hash: Vec<u8>, base_dir: &PathBuf) -> Result<(), String> {
        if self.hash.is_some() {
            // Project is not empty, cannot apply update
            return Err(format!("Project '{}' can not apply update, as it is in an unsuitable state", self.name.as_str()));
        }
        // Check if hash matches supplied Zipfile
        if !content.compare_hash(&hash) {
            return Err(String::from("Hashsum mismatch"));
        }
        // Try to extract content and apply update
        let mut dir = base_dir.clone();
        dir.push(self.name.as_str());
        let _ = content.extract_into(&dir)
            .await
            .map_err(|e| format!("Could not extract zip archive: {}", e))?;
        // Update hash
        self.hash = Some(hash);
        // Applied update successfully
        Ok(())
    }

    pub async fn execute_all_tests(&self, base_dir: &PathBuf) -> Result<Vec<TestOutput>, Box<dyn Error>> {
        let mut results = Vec::with_capacity(self.tests.len());
        for test in self.tests.iter() {
            let res = run_test(self.name.as_str(),
                test,
                base_dir
            ).await?;
            results.push(res);
        }
        Ok(results)
    }
}

impl From<crate::pb::Project> for TestProject {
    fn from(project: crate::pb::Project) -> Self {
        // FIXME: add proper error handling?
        let tests: Vec<Vec<String>> = project.tests.iter()
            .map(|s| shell_words::split(s).unwrap_or(Vec::new()) )
            .collect();
        TestProject {
            name: project.name,
            tests,
            hash: None,
        }
    }
}

impl From<TestProject> for crate::pb::Project {
    fn from(t: TestProject) -> Self {
        let tests: Vec<String> = t.tests.into_iter()
            .map(|v| shell_words::join(v))
            .collect();
        crate::pb::Project {
            name: t.name,
            tests,
        }
    }
}

async fn run_test(dirname: &str, command: &Vec<String>, base_dir: &PathBuf) -> Result<TestOutput, Box<dyn Error>> {
    let dir = {
        let mut d = base_dir.clone();
        d.push(dirname);
        d
    };
    let output = Command::new(&command[0])
        // Set working directory
        .current_dir(dir.as_path())
        .args(&command[1..])
        .stdin(Stdio::null())
        .output()
        .await?;
    // Return test run results
    Ok((
        shell_words::join(command),
        output.status.code(),
        output.stdout,
        output.stderr
    ))
}
