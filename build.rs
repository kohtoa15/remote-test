fn main() -> std::io::Result<()> {
    tonic_build::configure()
        .compile(
            &["./proto/remote-test.proto"],
            &["./proto"]
        )?;
    Ok(())
}
