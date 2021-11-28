use std::{error::Error, future::Future, path::Path, time::{Duration, Instant}};

use remote_test::{pb::{Project, ProjectIdentifier, ProjectUpdate, remote_client::RemoteClient}, zip::ZipBlob};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    pub tests: Vec<String>,
    pub exclude: Vec<String>,
}

impl From<&ProjectConfig> for Project {
    fn from(conf: &ProjectConfig) -> Self {
        Project { name: conf.name.clone(), tests: conf.tests.clone() }
    }
}

impl From<&ProjectConfig> for ProjectIdentifier {
    fn from(conf: &ProjectConfig) -> Self {
        ProjectIdentifier { name: conf.name.clone() }
    }
}

/// Reads project config struct from json config file
fn read_project_config(path: impl AsRef<Path>) -> Result<ProjectConfig, Box<dyn Error>> {
    let file = std::fs::File::open(path)?;
    let conf: ProjectConfig = serde_json::from_reader(file)?;
    Ok(conf)
}

async fn register_project(dest: String, conf: &ProjectConfig) -> Result<String, Box<dyn Error>> {
    let mut client = RemoteClient::connect(dest)
        .await?;
    let res = client.register_project(Project::from(conf))
        .await?
        .into_inner();
    let msg;
    if res.success {
        msg = format!("Successfully registered project {}", conf.name.as_str());
    } else if res.error.is_some() {
        msg = format!("Project could not be registered: {}", res.error.unwrap().as_str());
    } else {
        msg = String::from("Project could not be registered");
    }
    Ok(msg)
}

async fn unregister_project(dest: String, conf: &ProjectConfig) -> Result<String, Box<dyn Error>> {
    let mut client = RemoteClient::connect(dest)
        .await?;
    let res = client.unregister_project(ProjectIdentifier::from(conf))
        .await?
        .into_inner();
    let msg;
    if res.success {
        let mut buf = format!("Successfully unregistered project {}", conf.name.as_str());
        if let Some(e) = res.error {
            buf = format!("{}\n{}", buf.as_str(), e.as_str());
        }
        msg = buf;
    } else if res.error.is_some() {
        msg = format!("Project could not be unregistered: {}", res.error.unwrap().as_str());
    } else {
        msg = String::from("Project could not be unregistered");
    }
    Ok(msg)
}

async fn update_project(dest: String, conf: &ProjectConfig) -> Result<String, Box<dyn Error>> {
    let mut client = RemoteClient::connect(dest)
        .await?;
    let (hash, blob) = {
        let mut zip = ZipBlob::new(conf.exclude.clone())?;
        zip.add_dir(".").await?;
        zip.finish().await
    }?;
    let update = ProjectUpdate { name: conf.name.clone(), hash, blob};
    let res = client.update_project(update)
        .await?
        .into_inner();
    let msg;
    if res.success {
        msg = format!("{}:{} has been successsfully updated", res.project, res.hash);
    } else if res.error.is_some() {
        msg = format!("{}:{} could not be updated: {}", res.project, res.hash, res.error.unwrap());
    } else {
        msg = format!("{}:{} could not be updated", res.project, res.hash);
    }
    Ok(msg)
}

fn success_to_str(success: bool) -> &'static str {
    if success { "OK" } else { "Failed" }
}

async fn run_tests(dest: String, conf: &ProjectConfig) -> Result<String, Box<dyn Error>> {
    let mut client = RemoteClient::connect(dest)
        .await?;
    let res = client.run_tests(ProjectIdentifier::from(conf))
        .await?
        .into_inner();

    let all_successful = res.results.iter()
        .any(|x| !x.success);
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("Test results for {}:{}", res.name, res.hash));
    lines.push(format!(" started at {}", res.timestamp));
    for (i, result) in res.results.into_iter().enumerate() {
        let n = i + 1;
        lines.push(format!("Test {} {} {}", 
            n,
            "*".repeat(10),
            success_to_str(result.success)
        ));
        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        lines.push(format!("\tstdout: \"{}\"", stdout));
        lines.push(format!("\tstderr: \"{}\"", stderr));
    }
    lines.push(format!("{}", "*".repeat(20)));
    lines.push(format!("\tTests successful {} {}",
        "*".repeat(5),
        success_to_str(all_successful)
    ));
    Ok(lines.join("\n"))
}

fn help() {
    println!("Commands:");
    println!("\tregister\tRegister this project at our target server");
    println!("\tunregister\tUnregister (remove) this project at our target server");
    println!("\tinit\tUpdate inital project resources at our target server");
    println!("\trun\tRun tests at the remote server");
    println!("\thelp\tDisplays this text");
}

async fn print_result<Fut>(res: Fut) -> Result<(), Box<dyn Error>>
    where Fut: Future<Output = Result<String, Box<dyn Error>>>
{
    let join = tokio::spawn(async {
        use std::io::Write;
        loop {
            // Print dots in half second intervals
            let now = Instant::now();
            while now.elapsed() < Duration::from_millis(500) {
                tokio::task::yield_now().await;
            }
            print!(".");
            std::io::stdout().flush().unwrap();
        }
    });
    // Wait until task is ready, then abort dot-print task
    let result = res.await;
    join.abort();
    println!("");
    // Print result message
    match result {
        Ok(s) => {
            println!("{}", s);
            Ok(())
        },
        Err(e) => Err(e),
    }
}

#[tokio::main]
async fn main() {
    let config_file = option_env!("PROJECT_CONFIG").unwrap_or(".remotetest-config");
    let conf = read_project_config(config_file).expect("Could not read project config file");
    let dest = std::env::args().next().expect("You need to provide the destination host as argument");

    println!("### remote-test client {} ###", env!("CARGO_PKG_VERSION"));
    use std::io::Write;
    let mut buf = String::new();
    loop {
        buf.clear();
        print!("> ");
        std::io::stdout().flush().unwrap();
        let _n = std::io::stdin().read_line(&mut buf).unwrap();

        // Get input cmd
        match buf.as_str() {
            "register" => print_result(register_project(dest.clone(), &conf))
                .await
                .unwrap(),
            "unregister" => print_result(unregister_project(dest.clone(), &conf))
                .await
                .unwrap(),
            "init" => print_result(update_project(dest.clone(), &conf))
                .await
                .unwrap(),
            "run" => print_result(run_tests(dest.clone(), &conf))
                .await
                .unwrap(),
            "help" => help(),
            // Invalid command
            _ => println!("Invalid command. Enter 'help' to get more information on the commands"),
        }
    }
}
