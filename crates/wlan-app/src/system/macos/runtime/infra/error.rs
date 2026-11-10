impl From<security_framework::base::Error> for crate::infra::error::Error {
    fn from(error: security_framework::base::Error) -> Self {
        Self::external(error.into())
    }
}
