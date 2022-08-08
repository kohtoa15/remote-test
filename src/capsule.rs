use std::{error::Error, path::{Path, PathBuf}, process::{Stdio, Output}};

use async_trait::async_trait;
use log::{debug, trace};
use tokio::process::Command;

use crate::{project::TestOutput, zip::{ZipBlob, ZipFile}};


/// Provides a separate environment for tests to be run in
#[async_trait]
pub trait Capsule {
    type Error;
    type Args;

    async fn encapsulate(&mut self, ident: String, dir: &Path, args: Self::Args) -> Result<(), Self::Error>;

    async fn run_test(&self, cmd: &Vec<String>) -> Result<TestOutput, Self::Error>; 

    async fn discard(self) -> Result<(), Self::Error>;
}

/// Transparent capsule, same execution as if without
pub struct TransparentCapsule {
    dir: Option<PathBuf>
}

/// Capsule wrapper for test environments without extra encapsulation
#[async_trait]
impl Capsule for TransparentCapsule {
    type Error = Box<dyn Error>;
    type Args = ();

    async fn encapsulate(&mut self, _: String, dir: &Path, _: Self::Args) -> Result<(), Self::Error> {
        // Don't encapsulate
        self.dir.replace(dir.to_path_buf());
        Ok(())
    }

    async fn run_test(&self, cmd: &Vec<String>) -> Result<TestOutput, Self::Error> {
        if let Some(dir) = &self.dir {
            let output = Command::new(&cmd[0])
                // Set working directory
                .current_dir(dir.as_path())
                .args(&cmd[1..])
                .stdin(Stdio::null())
                .output()
                .await?;
            debug!("executed test '{}' -> {}",
                shell_words::join(cmd),
                output.status.code().map(|x| x.to_string()).unwrap_or("None".to_string()),
            );
            // Return test run results
            Ok((
                shell_words::join(cmd),
                output.status.code(),
                output.stdout,
                output.stderr
            ))
        } else {
            Err(Box::new(std::io::Error::new(std::io::ErrorKind::NotFound, "Directory not found")))
        }
    }

    async fn discard(self) -> Result<(), Self::Error> {
        // Nothing to discard
        Ok(())
    }
}

macro_rules! init_and_setter {
    ($x:ident, $set_x:ident, $t:ty) => {
        pub fn $set_x(&mut self, value: $t) -> &mut Self {
            let _ = self.$x.replace(value);
            self
        }

        pub fn $x(value: $t) -> Self {
            let mut k = Self::default();
            k.$set_x(value);
            return k;
        }
    };
}

#[derive(Debug, Clone)]
pub struct PodOptions {
    /// image used by pods
    image: Option<String>,
    /// ports to expose
    ports: Option<Vec<u16>>,
    // TODO: add more options
}

impl Default for PodOptions {
    fn default() -> Self {
        PodOptions { image: None, ports: None }
    }
}

impl PodOptions {
    // auto-generate init and setter methods
    init_and_setter!(image, set_image, String);
    init_and_setter!(ports, set_ports, Vec<u16>);

    /// Merge other instance of PodOptions into this one
    /// (Other one has preference)
    pub fn merge(&mut self, other: &Self) -> &mut Self {
        if let Some(image) = other.image.clone() {
            self.set_image(image);
        }
        if let Some(ports) = other.ports.clone() {
            self.set_ports(ports);
        }
        self
    }

    /// Print options as strings for use as kubectl options
    pub fn as_args_str(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(image) = &self.image {
            args.push(format!("--image={}", image.as_str()));
        }
        if let Some(ports) = &self.ports {
            for port in ports {
                args.push(format!("--port={}", *port));
            }
        }
        args
    }
}

/// Testwise encapsulation via k8s pods
///
/// Starts up separate pods for each test run
pub struct KubernetesCapsule {
    // Defaults to kubectl, could be specific executable
    program: Option<String>,
    // Auth token for k8s
    token: String,
    // Namespace used to start up pods
    namespace: String,
    // base config for running pods
    options: PodOptions,
    // identifier for spawned pod
    podname: Option<String>,
}

impl KubernetesCapsule {
    fn program(&self) -> &str {
        if let Some(p) = &self.program {
            p.as_str()
        } else {
            "kubectl"
        }
    }

    async fn kube_cmd(&self, args: &Vec<String>) -> Result<Output, Box<dyn Error>> {
        debug!("{} {}", self.program(), args.join(" "));
        let output = Command::new(self.program())
            .args(&args[..])
            .stdin(Stdio::null())
            .output()
            .await?;
        trace!("stdout: '{:?}'\nstderr: '{:?}'", output.stdout, output.stderr);
        let code = output.status.code().unwrap_or(-1);
        debug!("status: {}", code);
        Ok(output)
    }

    // runs kube_cmd without return code return
    async fn kube_cmd_silent(&self, args: &Vec<String>) -> Result<(), Box<dyn Error>> {
        let output = self.kube_cmd(&args).await?;
        if output.status.code().is_some() && output.status.code().unwrap_or(-1) == 0 {
            Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, format!("kubectl {} return code: {}", args.join(" "), output.status.code().unwrap_or(-1)))))
        } else {
            Ok(())
        }
    }

    async fn kube_run(&self) -> Result<(), Box<dyn Error>> {
        let mut args = self.options.as_args_str();
        // Insert kubectl subcmd and podname in front
        args.insert(0, String::from("run"));
        args.insert(1, self.podname.clone().unwrap());
        self.kube_cmd_silent(&args).await
    }

    async fn kube_cp(&self, mut src: String, mut dest: String, to_pod: bool) -> Result<(), Box<dyn Error>> {
        if let Some(podname) = self.podname.clone() {
            if to_pod {
                dest = format!("{}:{}", podname.as_str(), dest);
            } else {
                src = format!("{}:{}", podname.as_str(), src);
            }
            let args = vec![
                "cp".to_string(),
                src,
                dest,
            ];
            self.kube_cmd_silent(&args).await
        } else {
            panic!("No podname defined");
        }
    }

    async fn kube_exec(&self, p_args: &Vec<String>) -> Result<Output, Box<dyn Error>> {
        if let Some(podname) = self.podname.clone() {
            // insert exec cmd items
            let mut args = Vec::new();
            args.push("exec".to_string());
            args.push(podname);
            args.push("--".to_string());
            args.append(&mut p_args.clone());
            self.kube_cmd(&args).await
        } else {
            panic!("No podname defined");
        }
    }

    async fn kube_delete_pod(&self) -> Result<(), Box<dyn Error>> {
        if let Some(podname) = self.podname.clone() {
            let mut args = Vec::new();
            args.push("delete".to_string());
            args.push("pod".to_string());
            args.push(podname);
            self.kube_cmd_silent(&args).await
        } else {
            panic!("No podname defined");
        }
    }
}

#[async_trait]
impl Capsule for KubernetesCapsule {
    // TODO: create specific Error type for layering
    type Error = Box<dyn Error>;
    type Args = PodOptions;

    /// Use to start up pod
    async fn encapsulate(&mut self, ident: String, dir: &Path, args: Self::Args) -> Result<(), Self::Error> {
        self.podname = Some(format!("rtk-capsule-{}", ident.as_str()));
        let options = self.options.clone()
            .merge(&args);
        // Create pod with options
        self.kube_run()
            .await?;
        // copy files to pod
        let src = dir.to_string_lossy();
        self.kube_cp(src.to_string(), String::from("/etc/repo/"), true)
            .await?;
        Ok(())
    }

    /// 
    async fn run_test(&self, cmd: &Vec<String>) -> Result<TestOutput, Self::Error> {
        let output = self.kube_exec(cmd).await?;
        return Ok((cmd.join(" "), output.status.code(), output.stdout, output.stderr));
    }

    async fn discard(self) -> Result<(), Self::Error> {
        // Delete running pod
        self.kube_delete_pod().await
    }
}
