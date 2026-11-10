impl From<zbus::Error> for crate::infra::error::Error {
    fn from(error: zbus::Error) -> Self {
        Self::external(error.into())
    }
}
impl From<zbus::zvariant::Error> for crate::infra::error::Error {
    fn from(error: zbus::zvariant::Error) -> Self {
        Self::external(error.into())
    }
}
impl From<dbus_secret_service::Error> for crate::infra::error::Error {
    fn from(error: dbus_secret_service::Error) -> Self {
        Self::external(error.into())
    }
}
