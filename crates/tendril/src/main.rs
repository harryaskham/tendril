fn main() -> anyhow::Result<()> {
    tendril::run(std::env::args_os())?;
    Ok(())
}
