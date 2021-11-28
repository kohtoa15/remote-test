use std::{collections::HashMap, net::SocketAddr, path::{Path, PathBuf}, sync::Arc};

use log::{debug, error, info, trace};
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
        trace!("received RegisterRequest for project '{}'", name.as_str());
        let mut p = self.projects.write().await;

        // Insert new project if name does not yet exist
        if p.contains_key(&name) {
            trace!("project {} already exists", name.as_str());
            response!(RegisterResponse {
                success: false,
                error: Some(format!("Project with name '{}' already exists!", name.as_str())),
            })
        } else {
            debug!("successfully registered project {}", name.as_str());
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
        let project_name = request.into_inner().name;
        trace!("received UnregisterRequest for project '{}'", project_name.as_str());
        let mut p = self.projects.write().await;


        // Try to remove project, if it exists
        match p.remove(&project_name) {
            Some(project) => {
                trace!("unregistering project {}", project_name.as_str());
                let mut error = None;
                // Clear project repo
                let dir = project.get_dir(&self.base_dir);
                if dir.exists() && dir.is_dir() {
                    trace!("removing project folder {:?}", dir.as_os_str());
                    if let Err(e) = tokio::fs::remove_dir_all(dir.as_path()).await {
                        error!("could not clear directory {}", e.to_string());
                        error = Some(format!("Could not clear directory: {}", e));
                    };
                }
                debug!("successfully unregistered project {}", project_name.as_str());
                response!(RegisterResponse { success: true, error })
            },
            None => {
                trace!("project {} does not exist", project_name.as_str());
                response!(RegisterResponse { success: false, error: Some(format!("Project '{}' does not exist", project_name.as_str())) })
            },
        }
    }

    async fn update_project(
        &self,
        request: Request<ProjectUpdate>
    ) -> Result<Response<UpdateResponse>,Status> {
        let update = request.into_inner();
        trace!("received ProjectUpdate for project {}", update.name.as_str());
        // Check that project exists and currently has no hash
        let mut p = self.projects.write().await;
        match p.get_mut(&update.name) {
            Some(project) => {
                trace!("preparing update for project {}", update.name.as_str());
                // Store content to local file
                let zipfile = ZipFile::from_contents(update.blob, &self.zip_cache_dir)
                    .await
                    .map_err(|e| {
                        error!("error occurred while trying to write blob to file: {}", e);
                        Status::aborted(format!("Error occurred while trying to write blob to file: {}", e))
                    })?;
                let hash = base64::decode_config(&update.hash, base64::STANDARD)
                    .map_err(|e| {
                        error!("could not decode provided update hash: {}", e);
                        Status::aborted(format!("Could not decode provided update hash: {}", e))
                    })?;
                match project.apply_update(zipfile, hash, &self.base_dir)
                .await {
                    Ok(_) => {
                        debug!("applied update {} to project {}", update.hash.as_str(), update.name.as_str());
                        response!(UpdateResponse {
                            project: update.name, 
                            hash: update.hash,
                            success: true,
                            error: None,
                        })
                    },
                    Err(e) => {
                        trace!("could not apply update {} to project {}: {}", update.hash.as_str(), update.name.as_str(), e.to_string());
                        response!(UpdateResponse {
                            project: update.name,
                            hash: update.hash,
                            success: false,
                            error: Some(e)
                        })
                    },
                }
            },
            // no project with this name
            None => {
                trace!("project {} does not exist", update.name.as_str());
                response!(UpdateResponse {
                    error: Some(format!("Project '{}' does not exist", update.name.as_str())),
                    project: update.name,
                    hash: update.hash,
                    success: false,
                })
            },
        }
    }
    
    async fn increment_project(
        &self,
        _request: Request<ProjectIncrement>
    ) ->Result<Response<UpdateResponse>,Status> {
        // TODO: Add incremental update procedure
        error!("client tried to apply ProjectIncrement, which is unimplemented!");
        Err(Status::unimplemented("Not yet implemented!"))
    }

    async fn run_tests(
        &self,
        request: Request<ProjectIdentifier>
    ) -> Result<Response<TestResults>,Status> {
        let project = request.into_inner().name;
        trace!("received RunTest request for project {}", project.as_str());

        // Generate pre-test timestamp
        let timestamp = chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        // Get project info
        let p = self.projects.read().await;
        let test_project = p.get(&project)
            .ok_or({
                trace!("project {} does not exist", project.as_str());
                Status::invalid_argument(format!("Project '{}' does not exist!", project.as_str()))
            })?;

        // Run all configured tests for project
        let results = test_project.execute_all_tests(&self.base_dir)
            .await
            .map(|v| v.into_iter()
                .map(|k| TestResult::from(k))
                .collect()
            )
            .map_err(|e| {
                error!("error occured while running test: {}", e);
                Status::aborted(format!("Error occurred while running test: {}", e))
            })?;

        // Return test results
        let (name, hash) = test_project.get_tuple();
        debug!("Ran tests for project {}:{}", name.as_str(), hash.as_str());
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

static LOGGER: SimpleLogger = SimpleLogger;

struct SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        // let all logs pass
        true
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            println!(
                "{} [{}]: {} - {}",
                timestamp.as_str(),
                record.level(),
                record.target(),
                record.args(),
            );
        }
    }

    fn flush(&self) {}
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

    // Prepare logger
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Trace);

    let port = u16::from_str_radix(option_env!("PORT").unwrap_or("19000"), 10).expect("Could not parse port number");
    let host = SocketAddr::from(([127, 0, 0, 1], port));
    info!("Starting server at {}", host);
    let server = RemoteServerContext::new(repo_dir, zip_cache_dir);
    Server::builder()
        .add_service(RemoteServer::new(server))
        .serve(host)
        .await
        .unwrap();
}
