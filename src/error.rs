#[derive(thiserror::Error, Debug)]
pub enum WaftError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
