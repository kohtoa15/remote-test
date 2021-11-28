use std::{collections::HashMap, net::SocketAddr, path::{Path, PathBuf}, sync::Arc};

use remote_test::{pb::{Project, ProjectIdentifier, ProjectIncrement, ProjectUpdate, RegisterResponse, TestResult, TestResults, UpdateResponse, remote_server::{Remote, RemoteServer}}, project::TestProject, zip::ZipFile};
use tokio::{fs::DirBuilder, sync::RwLock};
use tonic::{Request, Response, Status, transport::Server};

macro_rules! response {
    ($x:expr) => {
        Ok(tonic::Response::new($x))
    };
}

pub struct RemoteServerContext {
    base_dir: PathBuf,
    zip_cache_dir: PathBuf,
    projects: Arc<RwLock<HashMap<String, TestProject>>>,
}

impl RemoteServerContext {
    pub fn new(base_dir: PathBuf, zip_cache_dir: PathBuf) -> Self {
        RemoteServerContext {
            base_dir,
            zip_cache_dir,
            projects: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[tonic::async_trait]
impl Remote for RemoteServerContext {
    async fn register_project(
        &self,
        request: Request<Project>
    ) -> Result<Response<RegisterResponse>,Status> {
        let project: TestProject = request.into_inner().into();
        let name = project.get_name().to_string();
        let mut p = self.projects.write().await;

        // Insert new project if name does not yet exist
        if p.contains_key(&name) {
            response!(RegisterResponse {
                success: false,
                error: Some(format!("Project with name '{}' already exists!", name.as_str())),
            })
        } else {
            let _ = p.insert(name, project);
            response!(RegisterResponse {
                success: true,
                error: None,
            })
        }
    }

    async fn unregister_project(
        &self,
        request: Request<ProjectIdentifier>
    ) ->Result<Response<RegisterResponse>,Status> {
        let project = request.into_inner().name;
        let mut p = self.projects.write().await;


        // Try to remove project, if it exists
        match p.remove(&project) {
            Some(project) => {
                let mut error = None;
                // Clear project repo
                let dir = project.get_dir(&self.base_dir);
                if dir.exists() && dir.is_dir() {
                    if let Err(e) = tokio::fs::remove_dir_all(dir.as_path()).await {
                        error = Some(format!("Could not clear directory: {}", e));
                    };
                }
                response!(RegisterResponse { success: true, error })
            },
            None => response!(RegisterResponse { success: false, error: Some(format!("Project '{}' does not exist", project.as_str())) })
        }
    }

    async fn update_project(
        &self,
        request: Request<ProjectUpdate>
    ) -> Result<Response<UpdateResponse>,Status> {
        let update = request.into_inner();
        // Check that project exists and currently has no hash
        let mut p = self.projects.write().await;
        match p.get_mut(&update.name) {
            Some(project) => {
                // Store content to local file
                let zipfile = ZipFile::from_contents(update.blob, &self.zip_cache_dir)
                    .await
                    .map_err(|e| Status::aborted(format!("Error occurred while trying to write blob to file: {}", e)))?;
                let hash = base64::decode_config(&update.hash, base64::STANDARD)
                    .map_err(|e| Status::aborted(format!("Could decode provided update hash: {}", e)))?;
                match project.apply_update(zipfile, hash, &self.base_dir)
                .await {
                    Ok(_) => response!(UpdateResponse {
                        project: update.name, 
                        hash: update.hash,
                        success: true,
                        error: None,
                    }),
                    Err(e) => response!(UpdateResponse {
                        project: update.name,
                        hash: update.hash,
                        success: false,
                        error: Some(e)
                    }),
                }
            },
            // no project with this name
            None => response!(UpdateResponse {
                error: Some(format!("Project '{}' does not exist", update.name.as_str())),
                project: update.name,
                hash: update.hash,
                success: false,
            }),
        }
    }
    
    async fn increment_project(
        &self,
        _request: Request<ProjectIncrement>
    ) ->Result<Response<UpdateResponse>,Status> {
        // TODO: Add incremental update procedure
        Err(Status::unimplemented("Not yet implemented!"))
    }

    async fn run_tests(
        &self,
        request: Request<ProjectIdentifier>
    ) -> Result<Response<TestResults>,Status> {
        let project = request.into_inner().name;

        // Generate pre-test timestamp
        let timestamp = chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        // Get project info
        let p = self.projects.read().await;
        let test_project = p.get(&project)
            .ok_or(Status::invalid_argument(format!("Project '{}' does not exist!", project.as_str())))?;

        // Run all configured tests for project
        let results = test_project.execute_all_tests(&self.base_dir)
            .await
            .map(|v| v.into_iter()
                .map(|k| TestResult::from(k))
                .collect()
            )
            .map_err(|e| Status::aborted(format!("Error occurred while running test: {}", e)))?;

        // Return test results
        let (name, hash) = test_project.get_tuple();
        response!(TestResults {
            name,
            hash,
            timestamp,
            results,
        })
    }
}

async fn prepare_directory(dir: &str) -> Result<PathBuf, String> {
    let path = Path::new(dir).to_path_buf();
    if !path.exists() {
        // Try to create directory
        DirBuilder::new()
            .recursive(true)
            .create(dir)
            .await
            .map_err(|e| format!("Could not create directory {}: {}", dir, e))?;
    }
    if path.exists() && !path.is_dir() {
        Err(format!("{} exists, but is not a directory!", dir))
    } else {
        // Directory is present
        Ok(path)
    }
}

static DEFAULT_REPO_DIR: &'static str = "/var/remote-test";
static DEFAULT_ZIP_CACHE_DIR: &'static str = "/tmp/.remote-test_zip-cache.d";

#[tokio::main]
async fn main() {
    let repo_dir = prepare_directory(std::env::var("REPO_DIR").unwrap_or(DEFAULT_REPO_DIR.to_string()).as_str())
        .await
        .expect("Could not prepare REPO_DIR");
    let zip_cache_dir = prepare_directory(std::env::var("ZIP_CACHE_DIR").unwrap_or(DEFAULT_ZIP_CACHE_DIR.to_string()).as_str())
        .await
        .expect("Could not prepare ZIP_CACHE_DIR");

    let port = u16::from_str_radix(option_env!("PORT").unwrap_or("19000"), 10).expect("Could not parse port number");
    let host = SocketAddr::from(([127, 0, 0, 1], port));
    println!("Starting server at {}", host);
    let server = RemoteServerContext::new(repo_dir, zip_cache_dir);
    Server::builder()
        .add_service(RemoteServer::new(server))
        .serve(host)
        .await
        .unwrap();
}
