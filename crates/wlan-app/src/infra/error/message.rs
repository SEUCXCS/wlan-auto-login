//! Central English diagnostic templates. Messages do not imply a failed Result.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)] // Some messages are used only by other compilation targets.
pub enum Message {
    #[error("Failed to configure Keychain interaction: {status}")]
    KeychainInteractionFailed { status: i32 },
    #[error("Failed to delete Keychain credential: {status}")]
    KeychainDeleteFailed { status: i32 },
    #[error("Failed to establish trusted application identity: {status}")]
    KeychainTrustedApplicationFailed { status: i32 },
    #[error("Failed to create credential access control: {status}")]
    KeychainAccessCreateFailed { status: i32 },
    #[error("Failed to authorize the installed application to read credentials: {status}")]
    KeychainAuthorizeFailed { status: i32 },
    #[error("Unmanaged or redirected bundle member retained: {path}")]
    BundleUnmanagedMember { path: String },
    #[error("Unexpected bundle identity: {key}")]
    UnexpectedBundleIdentity { key: String },
    #[error("Installation rollback incomplete: {failures}; recovery files: {backup}")]
    BundleRollbackIncomplete { failures: String, backup: String },
    #[error(
        "System application directory must be owned by root, readable and traversable by users, and not writable by them: {path}"
    )]
    SystemApplicationDirectoryPermissions { path: String },

    #[error("Keychain operation lock is poisoned")]
    KeychainOperationLockPoisoned,
    #[error("Failed to open user Keychain")]
    FailedToOpenUserKeychain,
    #[error(
        "Saved credentials are inaccessible; unlock Keychain or allow WLAN Login during a foreground operation"
    )]
    KeychainCredentialInaccessible,
    #[error("Cannot update the existing credential; the original item has been retained")]
    KeychainUpdateRetainedCredential,
    #[error("Failed to save Keychain credentials")]
    KeychainSaveFailed,
    #[error("Failed to access credential for deletion; item retained")]
    KeychainDeleteAccessFailed,
    #[error("Failed to start Core Location run loop")]
    LocationRunLoopFailed,
    #[error("Location manager stopped")]
    LocationManagerStopped,
    #[error("Location authorization timed out; retry after responding to the system dialog")]
    LocationAuthorizationTimedOut,
    #[error("Wi-Fi host is already running")]
    MacosHostAlreadyRunning,
    #[error("Native Wi-Fi query timed out")]
    NativeWifiQueryTimedOut,
    #[error("Managed Wi-Fi hosts stop with their service")]
    ManagedHostStopsWithService,
    #[error("Wi-Fi helper did not stop; bundle retained")]
    MacosHostStopFailed,
    #[error("Bundle member escaped its root")]
    BundleMemberOutsideRoot,
    #[error("Non-UTF-8 bundle member")]
    BundleMemberNotUtf8,
    #[error("Bundle executable is missing")]
    BundleExecutableMissing,
    #[error("Service registration succeeded but the managed application did not become ready")]
    ManagedAppNotReady,
    #[error("Timed out authorizing installed Keychain identity")]
    KeychainAuthorizationTimedOut,
    #[error("Installed application cannot read its saved credentials silently")]
    InstalledCredentialCheckFailed,
    #[error("Installation failed; previous bundle, definition and credential access restored")]
    BundleInstallationRestored,
    #[error(
        "Location access is unavailable ({state}); allow WLAN Login in System Settings > Privacy & Security > Location Services"
    )]
    LocationUnavailable { state: String },
    #[error(
        "A signed wlanlogin.app is required; build it with cargo xtask bundle macos and run its Contents/MacOS/wlanlogin executable"
    )]
    MacosBundleRequired,
    #[error(
        "No active macOS user session; automatic authentication is waiting for a user to log in"
    )]
    MacosUserSessionUnavailable,
    #[error(
        "Wi-Fi host is unavailable or not ready; retry the operation in the logged-in user's session"
    )]
    MacosHostUnavailable,
    #[error("Wi-Fi host returned an invalid or incompatible response")]
    MacosHostProtocol,
    #[error("Wi-Fi host peer identity does not match the selected application and user")]
    MacosHostIdentity,
    #[error("Login cancelled")]
    LoginCancelled,
    #[error("Installed executable returned an unrecognized version")]
    InstalledExecutableReturnedAnUnrecognizedVersion,
    #[error("Timed out waiting for service stop; no new start request submitted")]
    TimedOutWaitingForServiceStopNoNew,
    #[error("Service is not installed; run install first")]
    ServiceNotInstalled,
    #[error("Service installation is incomplete; run install again")]
    InstallationIncomplete,
    #[error("Service is not registered; run install first")]
    ServiceNotRegistered,
    #[error("Service registration was removed; run install again")]
    ServiceRegistrationRemoved,
    #[error("Failed to read journal; check journalctl availability and access permissions")]
    FailedToReadJournalCheckJournalctlAvailabilityAnd,
    #[error("Failed to register Ctrl+C listener")]
    FailedToRegisterCtrlCListener,
    #[error("Installation path has no parent directory")]
    InstallationPathHasNoParentDirectory,
    #[error("Installation failed; installation files restored")]
    InstallationFailedInstallationFilesRestored,
    #[error("Service definition has no parent directory")]
    ServiceDefinitionHasNoParentDirectory,
    #[error("Backup path has no parent directory")]
    BackupPathHasNoParentDirectory,
    #[error("unexpected service command")]
    UnexpectedServiceCommand,
    #[error("systemd response is missing LoadState")]
    SystemdResponseIsMissingLoadState,
    #[error("systemd response is missing ActiveState")]
    SystemdResponseIsMissingActiveState,
    #[error("systemd response is missing SubState")]
    SystemdResponseIsMissingSubState,
    #[error("launchd response is missing state")]
    LaunchdResponseIsMissingState,
    #[error("Task Scheduler response has an invalid Installed field")]
    TaskSchedulerResponseHasAnInvalidInstalledField,
    #[error("Unknown Task Scheduler state")]
    UnknownTaskSchedulerState,
    #[error("Task Scheduler response has an invalid State field")]
    TaskSchedulerResponseHasAnInvalidStateField,
    #[error("SCM response is missing Installed")]
    SCMResponseIsMissingInstalled,
    #[error("Unknown SCM service state")]
    UnknownSCMServiceState,
    #[error("Service uninstalled but credential deletion failed; retry uninstall --purge")]
    ServiceUninstalledButCredentialDeletionFailedRetryUninstall,
    #[error(
        "Service uninstalled but installed executable could not be removed; close it before retrying"
    )]
    ServiceUninstalledButInstalledExecutableCouldNotBe,
    #[error("Service definition has no file name")]
    ServiceDefinitionHasNoFileName,
    #[error("Service link has no parent directory")]
    ServiceLinkHasNoParentDirectory,
    #[error("Failed to remove service autostart link")]
    FailedToRemoveServiceAutostartLink,
    #[error("Background credential read timed out; waiting for the pending read")]
    BackgroundCredentialReadTimedOutWaitingForThe,
    #[error("Background credential read task failed")]
    BackgroundCredentialReadTaskFailed,
    #[error("Failed to delete user credentials; unlock the system keyring")]
    FailedToDeleteUserCredentialsUnlockTheSystem,
    #[error("Failed to delete user credentials")]
    FailedToDeleteUserCredentials,
    #[error("Credential deletion task failed")]
    CredentialDeletionTaskFailed,
    #[error("Failed to open system keyring")]
    FailedToOpenSystemKeyring,
    #[error("Failed to read credentials; unlock the system keyring")]
    FailedToReadCredentialsUnlockTheSystemKeyring,
    #[error("Credential read task failed")]
    CredentialReadTaskFailed,
    #[error("Saved credentials are not UTF-8")]
    SavedCredentialsAreNotUTF8,
    #[error("Failed to save credentials; unlock the system keyring")]
    FailedToSaveCredentialsUnlockTheSystemKeyring,
    #[error("Credential save task failed")]
    CredentialSaveTaskFailed,
    #[error("Keyring is locked; retry after unlocking")]
    KeyringIsLockedRetryAfterUnlocking,
    #[error("Duplicate saved credentials found; remove duplicate entries from the system keyring")]
    DuplicateSavedCredentialsFoundRemoveDuplicateEntriesFrom,
    #[error("Invalid saved credential format")]
    InvalidSavedCredentialFormat,
    #[error("Saved credentials are incomplete; run login --remember-me again")]
    SavedCredentialsAreIncompleteRunLoginRememberMe,
    #[error("Wi-Fi identification failed; portal logout was not attempted")]
    WiFiIdentificationFailedPortalLogoutWasNot,
    #[error("Failed to read password from stdin")]
    FailedToReadPasswordFromStdin,
    #[error("Failed to start Wi-Fi worker thread")]
    FailedToStartWiFiWorkerThread,
    #[error("Wi-Fi worker channel closed")]
    WiFiWorkerChannelClosed,
    #[error("Wi-Fi worker thread stopped")]
    WiFiWorkerThreadStopped,
    #[error("Failed to create system network monitor")]
    FailedToCreateSystemNetworkMonitor,
    #[error("Failed to read Wi-Fi interface name")]
    FailedToReadWiFiInterfaceName,
    #[error("Failed to subscribe to system network events")]
    FailedToSubscribeToSystemNetworkEvents,
    #[error("Failed to read Wi-Fi SSID; grant location permission in System Settings")]
    FailedToReadWiFiSSIDGrantLocation,
    #[error("Invalid WLAN response length")]
    InvalidWLANResponseLength,
    #[error("Failed to connect to the system D-Bus")]
    FailedToConnectToTheSystemDBus,
    #[error("NetworkManager must be running")]
    NetworkManagerMustBeRunning,
    #[error("NetworkManager event listener stopped")]
    NetworkManagerEventListenerStopped,
    #[error("Wi-Fi monitor initialization timed out")]
    WiFiMonitorInitializationTimedOut,
    #[error("Wi-Fi status query timed out")]
    WiFiStatusQueryTimedOut,
    #[error("Failed to determine user configuration directory")]
    FailedToDetermineUserConfigurationDirectory,
    #[error("Invalid connection preferences")]
    InvalidConnectionPreferences,
    #[error("Failed to read connection preferences")]
    FailedToReadConnectionPreferences,
    #[error("File has no parent directory")]
    FileHasNoParentDirectory,
    #[error("Cleanup path has no parent directory")]
    CleanupPathHasNoParentDirectory,
    #[error("Managing system services and credentials requires an elevated terminal")]
    ElevationRequired,
    #[error(
        "Run --user commands from a non-elevated terminal to access that user's services and keyring"
    )]
    UserSessionRequired,
    #[error("Failed to register SIGTERM listener")]
    FailedToRegisterSIGTERMListener,
    #[error("Failed to register SIGINT listener")]
    FailedToRegisterSIGINTListener,
    #[error("Failed to register console close listener")]
    FailedToRegisterConsoleCloseListener,
    #[error("Failed to register Ctrl+Break listener")]
    FailedToRegisterCtrlBreakListener,
    #[error("Process shutdown signal channel closed")]
    ProcessShutdownSignalChannelClosed,
    #[error("Service manager command timed out")]
    ServiceManagerCommandTimedOut,
    #[error("Invalid daemon control state")]
    InvalidDaemonControlState,
    #[error("Daemon is already running")]
    DaemonIsAlreadyRunning,
    #[error("Another service management operation is in progress")]
    AnotherServiceManagementOperationIsInProgress,
    #[error("Another authentication operation is in progress; retry later")]
    AnotherAuthenticationOperationIsInProgressRetryLater,
    #[error("Daemon has not exited; stop the running daemon before retrying")]
    DaemonHasNotExitedStopTheRunningDaemon,
    #[error("Control socket path is occupied by another file")]
    ControlSocketPathIsOccupiedByAnotherFile,
    #[error("Network check timed out")]
    NetworkCheckTimedOut,
    #[error("Local control channel closed")]
    LocalControlChannelClosed,
    #[error("Save credentials with login --remember-me before starting the daemon")]
    DaemonCredentialsMissing,
    #[error("Saved credentials are missing; run login --remember-me again")]
    SavedCredentialsMissing,
    #[error(
        "Credentials rejected; automatic retries paused, update credentials with login --interactive --remember-me"
    )]
    CredentialsRejected,
    #[error("Failed to open System Keychain")]
    FailedToOpenSystemKeychain,
    #[error("Failed to find System Keychain credentials for deletion")]
    FailedToFindSystemKeychainCredentialsForDeletion,
    #[error("Failed to read System Keychain credentials; check service access permissions")]
    FailedToReadSystemKeychainCredentialsCheckService,
    #[error("Failed to save System Keychain credentials")]
    FailedToSaveSystemKeychainCredentials,
    #[error("DPAPI returned an empty credential payload")]
    DPAPIReturnedAnEmptyCredentialPayload,
    #[error(
        "Failed to run systemd-creds; encrypted credentials require a supported systemd version, or explicitly select --credential-store protected-file"
    )]
    FailedToRunSystemdCredsEncryptedCredentialsRequire,
    #[error("Failed to open credential input pipe")]
    FailedToOpenCredentialInputPipe,
    #[error("System credential encryption or decryption timed out")]
    SystemCredentialEncryptionOrDecryptionTimedOut,
    #[error(
        "System credentials are missing; run login --system --remember-me from an elevated terminal"
    )]
    SystemCredentialsAreMissingRunLoginSystemRemember,
    #[error("System credentials are not UTF-8")]
    SystemCredentialsAreNotUTF8,
    #[error("Credential cache lock poisoned")]
    CredentialCacheLockPoisoned,
    #[error("Failed to determine installation directory")]
    FailedToDetermineInstallationDirectory,
    #[error("Failed to determine user service directory")]
    FailedToDetermineUserServiceDirectory,
    #[error("systemd path must be UTF-8")]
    SystemdPathMustBeUTF8,
    #[error("Installation path contains control characters")]
    InstallationPathContainsControlCharacters,
    #[error("Installation path must be UTF-8")]
    InstallationPathMustBeUTF8,
    #[error("Log path must be UTF-8")]
    LogPathMustBeUTF8,
    #[error("Failed to determine service status: {value0}")]
    FailedToDetermineServiceStatus { value0: String },
    #[error("journalctl failed: {status}")]
    JournalctlFailedStatus { status: String },
    #[error("No service log entries available")]
    NoServiceLogEntriesAvailable,
    #[error("Service installed and started for scope {value0}; {value1}")]
    ServiceInstalledAndStartedForScope { value0: String, value1: String },
    #[error("Installation failed and rollback was incomplete: {value0}")]
    InstallationFailedAndRollbackWasIncomplete { value0: String },
    #[error("Service plan does not match the selected scope")]
    ServicePlanDoesNotMatchTheSelected,
    #[error(
        "Failed to connect to Windows Service Control Manager: {value0}; use daemon for direct execution"
    )]
    FailedToConnectToWindowsServiceControl { value0: String },
    #[error("Last task result: {value}")]
    LastTaskResultValue { value: String },
    #[error("Last exit code: {value}")]
    LastExitCodeValue { value: String },
    #[error("Last result: {value}")]
    LastResultValue { value: String },
    #[error("Refusing to remove an autostart link owned by another service: {value0}")]
    RefusingToRemoveAnAutostartLinkOwned { value0: String },
    #[error("Service uninstalled; saved credentials retained")]
    ServiceUninstalledSavedCredentialsRetained,
    #[error("Shared system logs and service audit records remain managed by the operating system")]
    SharedSystemLogsAndServiceAuditRecords,
    #[error("Service and application data removed for scope {value0}")]
    ServiceAndApplicationDataRemovedForScope { value0: String },
    #[error("Failed to disable background keychain prompts: {status}")]
    FailedToDisableBackgroundKeychainPromptsStatus { status: String },
    #[error("Username and password must not be empty")]
    UsernameAndPasswordMustNotBeEmpty,
    #[error("--credential-store is only supported for system credentials")]
    CredentialStoreIsOnlySupportedForSystem,
    #[error("Service installation requires saved credentials; Remember me is required")]
    ServiceInstallationRequiresSavedCredentialsRememberMe,
    #[error(
        "{notice}; service installation can continue and authentication will resume on Student_CX"
    )]
    NoticeServiceInstallationCanContinueAndAuthentication { notice: String },
    #[error("Logged out")]
    LoggedOut,
    #[error("Local credentials cleared and automatic login stopped")]
    LocalCredentialsClearedAndAutomaticLoginStopped,
    #[error("Full logout incomplete; automatic login remains paused: {value0}")]
    FullLogoutIncompleteAutomaticLoginRemainsPaused { value0: String },
    #[error("Failed to delete credentials: {error}")]
    FailedToDeleteCredentialsError { error: String },
    #[error("Saved credentials deleted for the selected scope")]
    SavedCredentialsDeletedForTheSelectedScope,
    #[error("Portal logout failed: {error}")]
    PortalLogoutFailedError { error: String },
    #[error("Failed to stop daemon: {error}")]
    FailedToStopDaemonError { error: String },
    #[error("Not connected to Student_CX; deleting local credentials without portal logout")]
    NotConnectedToStudentCXDeletingLocal,
    #[error("--service is only supported by Windows Service Control Manager")]
    ServiceIsOnlySupportedByWindowsService,
    #[error("Credentials saved without authentication; credentials have not been verified")]
    CredentialsSavedOffline,
    #[error("Failed to read Wi-Fi SSID; check location permission: {code}")]
    FailedToReadWiFiSSIDCheck { code: String },
    #[error("Failed to enumerate Wi-Fi interfaces: {code}")]
    FailedToEnumerateWiFiInterfacesCode { code: String },
    #[error("Failed to subscribe to WLAN events: {result}")]
    FailedToSubscribeToWLANEventsResult { result: String },
    #[error("WlanOpenHandle failed: {result}")]
    WlanOpenHandleFailedResult { result: String },
    #[error(
        "Wi-Fi identification unavailable ({reason}); credentials will be saved without authentication"
    )]
    WiFiIdentificationUnavailableReasonCredentialsWill { reason: String },
    #[error("Failed to resolve system directory: {result}")]
    FailedToResolveSystemDirectoryResultX { result: String },
    #[error("Configuration directory is not owned by the current identity: {value0}")]
    ConfigurationDirectoryIsNotOwnedByThe { value0: String },
    #[error("Configuration directory must not be a symbolic link: {value0}")]
    ConfigurationDirectoryMustNotBeASymbolic { value0: String },
    #[error("Failed to remove empty directory: {value0}")]
    FailedToRemoveEmptyDirectory { value0: String },
    #[error("Failed to remove file: {value0}")]
    FailedToRemoveFile { value0: String },
    #[error("Failed to inspect directory: {value0}")]
    FailedToInspectDirectory { value0: String },
    #[error("Refusing to clean data through a reparse point: {value0}")]
    RefusingToCleanDataThroughAReparse { value0: String },
    #[error("Refusing to clean data through a non-directory or symbolic link: {value0}")]
    RefusingToCleanDataThroughANon { value0: String },
    #[error("Failed to initialize logging: {error}")]
    FailedToInitializeLoggingError { error: String },
    #[error("Daemon stopped")]
    DaemonStopped,
    #[error("Command {value0} {value1} failed: {value2}")]
    CommandFailed {
        value0: String,
        value1: String,
        value2: String,
    },
    #[error("Service log has not been created")]
    ServiceLogHasNotBeenCreated,
    #[error("Log path is not a regular file: {value0}")]
    LogPathIsNotARegularFile { value0: String },
    #[error(
        "Failed to read service log {value0}; check file permissions or use an elevated terminal for system logs"
    )]
    FailedToReadServiceLogCheckFile { value0: String },
    #[error("Failed to generate control token: {e}")]
    FailedToGenerateControlTokenE { e: String },
    #[error("Portal authentication state is unknown; skipping login")]
    PortalStateUnknown,
    #[error("Failed to apply system credential access control: {result}")]
    FailedToApplySystemCredentialAccessControl { result: String },
    #[error("Failed to create system credential access control: {result}")]
    FailedToCreateSystemCredentialAccessControl { result: String },
    #[error("Failed to authorize the service executable to read system credentials: {result}")]
    FailedToAuthorizeTheServiceExecutableTo { result: String },
    #[error("Failed to delete System Keychain credentials: {code}")]
    FailedToDeleteSystemKeychainCredentialsCode { code: String },
    #[error("Credential payload exceeds the size limit")]
    CredentialPayloadExceedsTheSizeLimit,
    #[error("System credential {operation} failed; storage mode was not downgraded")]
    SystemCredentialOperationFailedStorageModeWas { operation: String },
    #[error("System credentials must be owned by root with mode 0600")]
    SystemCredentialsMustBeOwnedByRoot,
    #[error("Credential path must be a regular file")]
    CredentialPathMustBeARegularFile,
    #[error(
        "System credentials will be stored as plaintext protected by administrator-only file permissions"
    )]
    SystemCredentialsWillBeStoredAsPlaintext,
    #[error("Plaintext machine credentials are not supported on Windows")]
    PlaintextMachineCredentialsAreNotSupportedOn,
    #[error(
        "protected-file is only available on Linux; this platform uses native machine credentials"
    )]
    ProtectedFileIsOnlyAvailableOnLinux,
    #[error(
        "Existing credentials use protected-file; back up and remove them before installing with native mode"
    )]
    ExistingCredentialsUseProtectedFileBackUp,
    #[error("Existing credentials are encrypted; keep native mode to avoid a plaintext downgrade")]
    ExistingCredentialsAreEncryptedKeepNativeMode,
    #[error("Conflicting system credential stores found; remove obsolete credentials first")]
    ConflictingSystemCredentialStoresFoundRemoveObsolete,
    #[error("System credential deletion failed: {value0}")]
    SystemCredentialDeletionFailed { value0: String },
    #[error("Failed to inspect installation file: {value0}")]
    FailedToInspectInstallationFile { value0: String },
    #[error(
        "Refusing to replace or remove a service definition owned by another application: {value0}"
    )]
    RefusingToReplaceOrRemoveAService { value0: String },
    #[error("Failed to resolve the original sudo user's installation")]
    FailedToResolveTheOriginalSudoUser,
    #[error(
        "A service is installed in another scope: {value0}; use its scope or uninstall it before switching scopes"
    )]
    AServiceIsInstalledInAnotherScope { value0: String },
    #[error("Daemon started")]
    DaemonStarted,
    #[error("Credentials temporarily unavailable")]
    CredentialsUnavailable,
    #[error("Wi-Fi monitor unavailable")]
    MonitorUnavailable,
    #[error("Network check failed")]
    CheckFailed,
    #[error("Network event subscription disconnected")]
    MonitorDisconnected,
    #[error("Authentication succeeded")]
    Authenticated,
    #[error("Already online; supplied credentials were not verified")]
    AlreadyOnline,
    #[error("Operation failed")]
    OperationFailed,
    #[error("Failed to create async runtime")]
    RuntimeCreationFailed,
    #[error("Unknown permission: {id}")]
    UnknownPermission { id: String },
    #[error(
        "Credentials are missing; provide arguments, WLANLOGIN_USERNAME/WLANLOGIN_PASSWORD, or an interactive terminal"
    )]
    CredentialsRequired,
    #[error("Authenticated but failed to save credentials; service installation cannot continue")]
    SaveAuthenticatedCredentialsFailed,
    #[error("Failed to save offline credentials; service installation cannot continue")]
    SaveOfflineCredentialsFailed,
    #[error("Service log output pipe is unavailable")]
    LogOutputUnavailable,
    #[error("Service entry is not initialized")]
    ServiceEntryUnavailable,
    #[error("Not connected to Student_CX; credentials will be saved without authentication")]
    NotConnectedToTargetWifi,
    #[error("{0}")]
    Detail(String),
}

impl From<String> for Message {
    fn from(value: String) -> Self {
        Self::Detail(value)
    }
}
impl From<&str> for Message {
    fn from(value: &str) -> Self {
        Self::Detail(value.into())
    }
}
