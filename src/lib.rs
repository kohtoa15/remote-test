pub mod client_errors;
pub mod project;
pub mod zip;

pub mod pb {
    tonic::include_proto!("grpc.remotetest");
}
pub mod hash {
    use std::sync::Arc;
    use lazy_static::lazy_static;
    use sha2::Digest;
    use tokio::sync::Mutex;

    lazy_static! {
        static ref HASHER: Arc<Mutex<sha2::Sha256>> = Arc::new(Mutex::new(sha2::Sha256::default()));
    }

    pub async fn hash(data: impl AsRef<[u8]>) -> String {
        let mut hasher = HASHER.lock().await;
        // Reset hasher after use, trust it's always used this way
        hasher.update(data);
        let res = hasher.finalize_reset();
        base64::encode_config(res.to_vec(), base64::STANDARD)
    }
}
